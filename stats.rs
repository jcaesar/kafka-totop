use crate::uses::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Topic {
    pub name: String,
    // The idea here is that I often test stuff in docker containers, and tend to delete those and start afresh quite often
    // That results in several unrelated instances of a topic with the same name.
    pub stat_idx: usize,
}

pub struct Stats {
    data: HashMap<String, Vec<TopicData>>,
    offrx: Receiver<scrape::Message>,
    scrape_interval: Duration, // This will get more complicated, with per-topic, variable intervals
    pub metadata_error: Option<KafkaError>,
}

#[derive(Default, Debug)]
pub struct TopicData {
    partitions: HashMap<i32, BTreeMap<Instant, i64>>,
    scraped_interval: Option<(Instant, Instant)>,
    decreased: usize,
    scraped: usize,
}

#[derive(Debug)]
pub struct TopicStats {
    /// Topic name
    pub topic: Topic,
    /// Sum of high watermarks
    pub total: i64,
    /// Difference of first and last sum of high watermarks
    pub seen: i64,
    /// seen / scrape_interval
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
                    let mut topdata = self
                        .data
                        .entry(topic)
                        .or_insert_with(|| Vec::with_capacity(1))
                        .back_or_push();
                    let partdata = topdata.partitions.entry(partition);
                    let decreased = match &partdata {
                        Entry::Occupied(partdata) => match partdata.get().values().rev().next() {
                            Some(latest) => *latest > offset,
                            None => false,
                        },
                        Entry::Vacant(_) => false,
                    };
                    if !decreased {
                        partdata.or_insert_with(BTreeMap::new).insert(now, offset);
                    } else {
                        topdata.decreased += 1;
                    }
                    topdata.scraped += 1;
                }
                Ok(scrape::Message::RoundFinished { now, topic }) => {
                    let topdatas = self.data.entry(topic).or_insert_with(Vec::default);
                    let topdata = topdatas.back_or_push();
                    if topdata.scraped > 0 {
                        if topdata.decreased <= topdata.partitions.len() / 2 {
                            topdata.scraped_interval.get_or_insert((now, now)).1 = now;
                            topdata.decreased = 0;
                            topdata.scraped = 0;
                            self.metadata_error = None;
                            update_display = true;
                        } else {
                            // This means we lose the first set of offsets if the topics were reset. Whatev.
                            topdatas.push(TopicData::default())
                        }
                    }
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
    pub fn basestats(&self) -> impl '_ + Iterator<Item = TopicStats> {
        self.data.iter().flat_map(|(topic, padata)| {
            padata.iter().rev().enumerate().map(|(idx, padata)| {
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
                    topic: Topic {
                        name: topic.to_owned(),
                        stat_idx: idx,
                    },
                    total,
                    seen,
                    rate,
                }
            })
        })
    }

    pub fn rates(
        &self,
        topic: &Topic,
        now: Instant,
        bucket_size: Duration,
    ) -> Option<Vec<(f64, f64)>> {
        let mut maxv = 1.0f64;
        let TopicData {
            partitions: padata,
            scraped_interval,
            ..
        } = self
            .data
            .get(&topic.name)?
            .iter()
            .rev()
            .nth(topic.stat_idx)?;
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
        for padatas in self.data.values_mut() {
            for TopicData { partitions, .. } in padatas {
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
}

trait BackOrPush {
    type Element;
    fn back_or_push(&mut self) -> &mut Self::Element;
}

impl<T: Default> BackOrPush for Vec<T> {
    type Element = T;

    fn back_or_push(&mut self) -> &mut Self::Element {
        if self.len() == 0 {
            self.push(Default::default());
        }
        self.iter_mut().rev().next().unwrap()
    }
}
