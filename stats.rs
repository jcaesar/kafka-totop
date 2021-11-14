use crate::uses::*;

pub struct Stats {
    data: HashMap<String, TopicData>,
    offrx: Receiver<scrape::Message>,
    scrape_interval: Duration, // This will get more complicated, with per-topic, variable intervals
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
    pub fn ingesting(offrx: Receiver<scrape::Message>, scrape_interval: Duration) -> Result<Self, anyhow::Error> {
        Ok(Self {
            offrx,
            data: HashMap::new(),
            scrape_interval,
        })
    }
    pub fn ingest(&mut self) -> Result<()> {
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
                    self.data.entry(topic)
                    .or_insert_with(TopicData::default)
                        .scraped_interval
                        .get_or_insert((now, now))
                        .1 = now;
                }
                Ok(msg) => anyhow::bail!("TODO: handle {:?}", msg),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
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
        interval: Duration,
        buckets: u32,
    ) -> (f64, Vec<(&str, Vec<(f64, f64)>, i64)>) {
        let now = Instant::now();
        let bucket_size = (interval / buckets).as_secs_f64();
        let draw_start = now - interval;
        let mut maxv = 1.0f64;
        let data = self
            .data
            .iter()
            .map(|(topic, padata)| {
                let mut buckets = (1 - (buckets as i32)..1)
                    .map(|idx| (bucket_size * idx as f64, 0f64))
                    .collect::<Vec<_>>();
                let mut total = 0;
                for (_, polls) in padata.partitions.iter() {
                    for ((ai, ao), (bi, bo)) in polls.iter().tuple_windows() {
                        let diff = bo - ao;
                        total += diff;
                        let aedge = ai.checked_duration_since(draw_start);
                        let bedge = bi.checked_duration_since(draw_start);
                        let dur = bi.checked_duration_since(*ai);
                        if let (Some(aedge), Some(bedge), Some(dur)) = (aedge, bedge, dur) {
                            let aidx = (aedge.as_secs_f64() / bucket_size) as usize;
                            let bidx = (bedge.as_secs_f64() / bucket_size) as usize;
                            if aidx == bidx {
                                if let Some((_, v)) = buckets.get_mut(bidx) {
                                    *v += diff as f64 / bucket_size;
                                    maxv = maxv.max(*v);
                                }
                            } else {
                                let rate = diff as f64 / dur.as_secs_f64();
                                if let Some((_, v)) = buckets.get_mut(aidx) {
                                    *v += rate
                                        * ((aidx + 1) as f64 - aedge.as_secs_f64() / bucket_size);
                                    maxv = maxv.max(*v);
                                }
                                if aidx + 1 <= bidx - 1 {
                                    for (_, v) in buckets[aidx + 1..=bidx - 1].iter_mut() {
                                        *v += rate;
                                        maxv = maxv.max(*v);
                                    }
                                }
                                if let Some((_, v)) = buckets.get_mut(bidx) {
                                    *v += rate * (bedge.as_secs_f64() / bucket_size - bidx as f64);
                                    maxv = maxv.max(*v);
                                }
                            }
                        }
                    }
                }
                (topic.as_str(), buckets, total)
            })
            .filter(|(_, _, total)| *total != 0)
            .collect::<Vec<_>>();
        (maxv, data)
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
