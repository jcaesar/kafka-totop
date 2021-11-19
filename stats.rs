use crate::uses::*;

pub struct Stats {
    data: HashMap<String, TopicData>,
    offrx: Receiver<scrape::Message>,
    scrape_interval: Duration, // This will get more complicated, with per-topic, variable intervals
    pub metadata_error: Option<KafkaError>,
}

#[derive(Default)]
pub struct TopicData {
    partitions: HashMap<i32, BTreeMap<Instant, i64>>,
    scraped_interval: Option<(Instant, Instant)>,
}

pub struct TopicStats<'a> {
    pub topic: &'a str,
    pub total: i64,
    pub seen: i64,
    pub rate: Option<f64>,
}

impl Stats {
    pub fn ingesting(
        offrx: Receiver<scrape::Message>,
        scrape_interval: Duration,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            offrx,
            data: HashMap::new(),
            scrape_interval,
            metadata_error: None,
        })
    }
    pub fn ingest(&mut self) -> Result<bool> {
        let mut update_display = false;
        loop {
            match self.offrx.try_recv() {
                Ok(scrape::Message::PartitionOffsets {
                    topic,
                    partition,
                    offset,
                    now,
                }) => {
                    self.data
                        .entry(topic)
                        .or_insert_with(TopicData::default)
                        .partitions
                        .entry(partition)
                        .or_insert_with(BTreeMap::new)
                        .insert(now, offset);
                }
                Ok(scrape::Message::RoundFinished { now, topic }) => {
                    self.data
                        .entry(topic)
                        .or_insert_with(TopicData::default)
                        .scraped_interval
                        .get_or_insert((now, now))
                        .1 = now;
                    self.metadata_error = None;
                    update_display = true;
                }
                Ok(scrape::Message::MetadataQueryFail(err)) => {
                    self.metadata_error = Some(err);
                    update_display = true;
                }
                Ok(msg) => anyhow::bail!("TODO: handle {:?}", msg),
                Err(mpsc::TryRecvError::Empty) => return Ok(update_display),
                Err(mpsc::TryRecvError::Disconnected) => {
                    // TODO: poll thread exit for an error for a second or so
                    anyhow::bail!("Metadata gatherer bailed");
                }
            }
        }
    }
    pub fn basestats(&self) -> impl Iterator<Item = TopicStats<'_>> {
        self.data.iter().map(|(topic, padata)| {
            let mut seen = 0;
            let mut total = 0;
            let mut rate = None;
            padata
                .partitions
                .values()
                .map(|polls| {
                    let first = polls.values().next()?;
                    let mut fromback = polls.iter().rev();
                    let (end, last) = fromback.next()?;
                    seen += last - first;
                    total += last;
                    let (pe, pl) = fromback.next()?;
                    *rate.get_or_insert(0.) +=
                        (last - pl) as f64 / end.duration_since(*pe).as_secs_f64();
                    Some(())
                })
                .for_each(|_| ());
            TopicStats {
                topic,
                total,
                seen,
                rate,
            }
        })
    }

    pub fn rates(
        &self,
        topic: &str,
        now: Instant,
        bucket_size: Duration,
    ) -> Option<Vec<(f64, f64)>> {
        let mut maxv = 1.0f64;
        let TopicData {
            partitions: padata,
            scraped_interval,
        } = self.data.get(topic)?;
        let (scrape_start, scrape_end) = scraped_interval.clone()?;
        if scrape_start > now {
            // Just avoid some WTFery
            return None;
        }
        let bucket_size_f = bucket_size.as_secs_f64();
        let mut buckets = (0..((scrape_end - scrape_start).as_secs_f64() / bucket_size_f) as usize)
            .map(|idx| {
                (
                    bucket_size_f * idx as f64
                        - ((now - scrape_start) + bucket_size / 2).as_secs_f64(),
                    0f64,
                )
            })
            .collect::<Vec<_>>();
        // TODO: tests... :(
        for (_, polls) in padata.iter() {
            for ((ai, ao), (bi, bo)) in polls.iter().tuple_windows() {
                let diff = bo - ao;
                let aedge = ai.checked_duration_since(scrape_start);
                let bedge = bi.checked_duration_since(scrape_start);
                let aidx = aedge.map(|aedge| (aedge.as_secs_f64() / bucket_size_f) as usize);
                let bidx = bedge.map(|bedge| (bedge.as_secs_f64() / bucket_size_f) as usize);
                let dur = *bi - *ai;
                if aidx == bidx {
                    if let Some((_, v)) = bidx.and_then(|bidx| buckets.get_mut(bidx)) {
                        *v += diff as f64 / bucket_size_f;
                        maxv = maxv.max(*v);
                    }
                } else {
                    let rate = diff as f64 / dur.as_secs_f64();
                    if let Some((_, v)) = aidx.and_then(|aidx| buckets.get_mut(aidx)) {
                        *v += rate
                            * ((aidx.unwrap() + 1) as f64
                                - aedge.unwrap().as_secs_f64() / bucket_size_f);
                        maxv = maxv.max(*v);
                    }
                    let aidx0 = aidx.map(|aidx| aidx + 1).unwrap_or(0);
                    let bidxm = bidx
                        .map(|bidx| bidx.saturating_sub(aidx0))
                        .unwrap_or(usize::MAX);
                    for (_, v) in buckets.iter_mut().skip(aidx0).take(bidxm) {
                        *v += rate;
                        maxv = maxv.max(*v);
                    }
                    if let Some((_, v)) = bidx.and_then(|bidx| buckets.get_mut(bidx)) {
                        *v += rate
                            * (bedge.unwrap().as_secs_f64() / bucket_size_f - bidx.unwrap() as f64);
                        maxv = maxv.max(*v);
                    }
                }
            }
        }
        Some(buckets)
    }

    pub fn discard_before(&mut self, discard: Instant) {
        let discard = match discard.checked_sub(self.scrape_interval) {
            Some(discard) => discard,
            None => return,
        };
        for TopicData { partitions, .. } in self.data.values_mut() {
            for polls in partitions.values_mut() {
                while let Some(first) = polls
                    .keys()
                    .next()
                    .map(Clone::clone)
                    .filter(|t| t < &discard)
                {
                    polls.remove(&first);
                }
            }
        }
    }
}
