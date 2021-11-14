use crate::uses::*;

#[derive(Debug)]
pub enum Message {
    MetadataQueryFail,
    BrokerQueryFail(String),
    PartitionOffsets {
        now: Instant,
        topic: String, // TODO: intern
        partition: i32,
        offset: i64,
    },
    RoundFinished {
        now: Instant,
        topic: String,
    },
}

struct QueryTask<'a> {
    when: Instant,
    topic: &'a str,
    partitions: &'a [MetadataPartition],
}

impl PartialEq for QueryTask<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when && self.topic == other.topic && self.partitions.len() == other.partitions.len() &&
        self.partitions.iter().zip(other.partitions.iter()).all(|(selb, other)| selb.id() == other.id() && selb.isr() == other.isr() && selb.leader() == other.leader() && selb.replicas() == other.replicas())
    }
}

impl Eq for QueryTask<'_> {}

impl PartialOrd for QueryTask<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueryTask<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.when.cmp(&other.when).then(self.topic.cmp(other.topic))
    }
}

#[derive(Default)]
pub struct State {
    bad_brokers: Mutex<HashSet<i32>>,
    query_interval: Duration,
}

type Client = rdkafka::admin::AdminClient<rdkafka::client::DefaultClientContext>;

pub fn spawn_threads(opts: &Opts) -> Result<Receiver<Message>> {
    let state = Arc::new(State {
        query_interval: opts.scrape_interval,
        ..State::default()
    });
    let (offtx, offrx) = mpsc::sync_channel(1_000_000);
    thread::spawn({
        let state = state.clone();
        let client = client(opts)?;
        || query_offsets(state, offtx, client)
    });
    thread::spawn({
        let state = state.clone();
        let client = client(opts)?;
        || query_bad(state, client)
    });
    Ok(offrx)
}

fn client(opts: &Opts) -> Result<Client> {
    let mut config = KafkaConfig::new();
    config.set("bootstrap.servers", &opts.brokers);
    for (k, v) in &opts.kafka_options {
        config.set(k, v);
    }
    let client: rdkafka::admin::AdminClient<_> =
        config.create().context("Failed to construct client")?;
    Ok(client)
}

fn query_offsets(state: Arc<State>, tx: mpsc::SyncSender<Message>, client: Client) -> Result<()> {
    let mut next = Instant::now();
    let client = client.inner();
    loop {
        next += state.query_interval;

        let mut query_tasks = BTreeSet::new(); // Better structures exist for scheduling.
                                               // TODO: Timeout probably shouldn't be the entire query interval...
        let metadata = client.fetch_metadata(None, state.query_interval);
        match metadata.as_ref() {
            Ok(metadata) => {
                let now = Instant::now();
                for topic in metadata.topics() {
                    let partitions = topic.partitions();
                    let offset = state
                        .query_interval
                        .mul_f64(rand_seeder::SipHasher::from(topic.name()).into_rng().gen());
                    query_tasks.insert(QueryTask {
                        when: now + offset,
                        topic: topic.name().into(),
                        partitions,
                    });
                }
            }
            Err(_) => (), // TODO?
        }

        let mut bads = state.bad_brokers.lock().expect("poisoned").clone();
        for QueryTask {
            when,
            topic,
            partitions,
        } in query_tasks.range(..)
        {
            for partition in partitions.iter() {
                if bads.contains(&partition.leader()) {
                    continue;
                }
                if let Some(sleep) = when.checked_duration_since(Instant::now()) {
                    thread::sleep(sleep);
                }
                match client.fetch_watermarks(topic, partition.id(), state.query_interval) {
                    Ok((_low, high)) => tx.send(Message::PartitionOffsets {
                        now: Instant::now(),
                        topic: (*topic).into(),
                        partition: partition.id(),
                        offset: high,
                    })?,

                    Err(_) => {
                        bads.insert(partition.leader());
                    }
                };
            }
            tx.send(Message::RoundFinished {
                now: Instant::now(),
                topic: (*topic).into(),
            })?;
        }
        // TODO: actually put bads back

        let now = Instant::now();
        if let Some(sleep) = next.checked_duration_since(now) {
            thread::sleep(sleep);
        } else {
            next = now;
        }
    }
}

fn query_bad(state: Arc<State>, client: Client) -> Result<()> {
    let mut next = Instant::now();
    let client = client.inner();
    loop {
        next += state.query_interval;

        let mut bads = state
            .bad_brokers
            .lock()
            .expect("poisoned")
            .drain()
            .map(|k| (k, false))
            .collect::<HashMap<_, _>>();
        match client.fetch_metadata(None, state.query_interval) {
            Ok(metadata) => {
                for topic in metadata.topics() {
                    for partition in topic.partitions() {
                        let leader = partition.leader();
                        match bads.entry(leader) {
                            Entry::Vacant(_) => (),                       // not bad
                            Entry::Occupied(entry) if *entry.get() => (), // already queried
                            Entry::Occupied(mut entry) => {
                                match client.fetch_watermarks(
                                    &topic.name(),
                                    partition.id(),
                                    state.query_interval,
                                ) {
                                    Ok(_) => entry.remove(),
                                    Err(_) => entry.insert(true),
                                };
                            }
                        }
                    }
                }
            }
            Err(_) => (),
        }
        *state.bad_brokers.lock().expect("poisoned") =
            bads.drain().filter(|(_, v)| *v).map(|(k, _)| k).collect();

        let now = Instant::now();
        if let Some(sleep) = next.checked_duration_since(now) {
            thread::sleep(sleep);
        } else {
            next = now;
        }
    }
}
