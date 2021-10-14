use structopt::StructOpt;
use anyhow::{Result, Context};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
    net::SocketAddr,
};
use rdkafka::{
    ClientConfig as KafkaConfig,
    client::Client,
};
use prometheus_exporter::prometheus::{
    register_int_counter_vec,
    register_int_gauge_vec,
};

/// Are my messages flowing?
#[derive(StructOpt, Debug)]
#[structopt(version = "0.1", author = "Julius Michaelis")]
struct Opts {
    /// Bootstrap broker address
    #[structopt(short, long)]
    brokers: String,
    /// Additional kafka client options
    #[structopt(short = "X", long, parse(try_from_str = parseopts))]
    kafka_options: Vec<(String, String)>,

    /// Polling interval
    #[structopt(short, long, default_value = "10 s", parse(try_from_str = parsehuman))]
    interval: Duration,

    /// Metadata retrieval timeout
    #[structopt(short, long, default_value = "5 s", parse(try_from_str = parsehuman))]
    timeout: Duration,

    /// How'd you like to view your data
    #[structopt(subcommand)]
    mode: Option<Mode>
}

#[derive(StructOpt, Debug)]
enum Mode {
    /// Start a prometheus server
    Export {
        /// Exporter listen address
        #[structopt(short, long, default_value = "0.0.0.0:9192")]
        listen: SocketAddr,
    },
    /// Just throw to stdout
    Print
}

fn parseopts(arg: &str) -> Result<(String, String)> {
    let mut split = arg.splitn(2, "=");
    if let (Some(k), Some(v), None) = (split.next(), split.next(), split.next()) {
        Ok((k.to_owned(), v.to_owned()))
    } else {
        anyhow::bail!("Expected a parameter of form config.key=value, got {}", arg);
    }
}

fn parsehuman(arg: &str) -> Result<Duration> {
    Ok(arg.parse::<humantime::Duration>().context(format!("Not a parsaeble time: {}", arg))?.into())
}

type PartitionData = HashMap<String, HashMap<i32, (i64, Instant)>>;
struct TopicData {
    name: String,
    total_messages: u64,
    current_rate: Option<f64>,
    underreplicated: u32,
    fetch_error: u32,
    partitions: u32,
}

fn fetchup(client: &Client, opts: &Opts, state: &mut PartitionData) -> Vec<TopicData> {
    let mut ret = vec![];
    match client.fetch_metadata(None, opts.timeout) {
        Ok(metadata) => {
            for topic in metadata.topics() {
                let name = topic.name().to_string();
                let offsets = state.entry(name.clone()).or_insert_with(HashMap::new);
                let mut fetch_error = 0_u32;
                let mut underreplicated = 0_u32;
                let mut current_rate = 0.0_f64;
                let mut total_messages = 0_u64;
                let mut new = true;
                for partition in topic.partitions() {
                    let id = partition.id();
                    if partition.isr().len() < partition.replicas().len() {
                        underreplicated += 1;
                    }
                    match client.fetch_watermarks(&name, id, opts.timeout) {
                        Ok((_low, high)) => {
                            let now = Instant::now();
                            total_messages += high as u64;
                            offsets.entry(id).and_modify(|(old_high, old_t)| {
                                current_rate += ((high - *old_high) as f64) / (now - *old_t).as_secs_f64();
                                *old_high = high;
                                *old_t = now;
                                new = false;
                            }).or_insert((high, now));
                        },
                        Err(_) => fetch_error += 1,
                    }
                }
                ret.push(TopicData {
                    total_messages, fetch_error, underreplicated, name,
                    current_rate: Some(current_rate).filter(|_| !new),
                    partitions: topic.partitions().len() as u32,
                });
            }
        },
        Err(e) => {
            eprintln!("Failed to query cluster, resting: {:?}", e);
        }
    }
    ret
}

fn main() -> Result<()> {
    let opts = Opts::from_args();
    let mut config = KafkaConfig::new();
    config.set("bootstrap.servers", &opts.brokers);
    for (k, v) in &opts.kafka_options {
        config.set(k, v);
    }
    let client: rdkafka::admin::AdminClient<_> = config.create()
        .context("Failed to construct client")?;
    let client = client.inner();
    let mut next = Instant::now() + opts.interval / 10;
    let mut offsetss = PartitionData::new();
    match opts.mode {
        None | Some(Mode::Print) => loop {
            println!("\n{}", chrono::Local::now());
            let mut data = fetchup(client, &opts, &mut offsetss);
            data.sort_by_key(|TopicData { total_messages, fetch_error, underreplicated, ..}| (*fetch_error, *underreplicated, *total_messages));
            for TopicData { name, current_rate, underreplicated, fetch_error, partitions, ..} in data {
                match current_rate {
                    None => if fetch_error != partitions {
                        println!("Topic found: {}", name);
                    },
                    Some(current_rate) => {
                        let output = format!("{}: {:.1} msg/s", name, current_rate);
                        let output = match fetch_error {
                            0 => output,
                            fetch_error => format!("{}, unavailable partitions: {}", output, fetch_error),
                        };
                        let output = match underreplicated {
                            0 => output,
                            underreplicated => format!("{}, underreplicated partitions: {}", output, underreplicated),
                        };
                        println!("{}", output);
                    }
                }
            }
            next += opts.interval;
            let now = Instant::now();
            if let Some(duration) = next.checked_duration_since(now) {
                std::thread::sleep(duration);
            }
        },
        Some(Mode::Export { listen }) => {
            let exporter = prometheus_exporter::start(listen)
                .context("Start prometheus listener")?;
            let m_total_messages = register_int_counter_vec!("kafka_light_message_count", "Sum of all high watermarks", &["topic"])?;
            let m_underreplicated = register_int_gauge_vec!("kafka_light_underreplicated", "Number of partitions where ISR != replicas", &["topic"])?;
            let m_fetch_error = register_int_gauge_vec!("kafka_light_fetch_error", "Number of partitions with metadata retrieval errors", &["topic"])?;
            loop {
                let _guard = exporter.wait_request();
                let now = Instant::now();
                if now > next {
                    next = now + opts.interval;
                    let data = fetchup(client, &opts, &mut offsetss);
                    for TopicData { name, total_messages, underreplicated, fetch_error, ..} in data {
                        let m_total_messages = m_total_messages.get_metric_with_label_values(&[&name])?;
                        m_total_messages.inc_by(total_messages - m_total_messages.get());
                        m_underreplicated.get_metric_with_label_values(&[&name])?.set(underreplicated as i64);
                        m_fetch_error.get_metric_with_label_values(&[&name])?.set(fetch_error as i64);
                    }
                }
            }
        }
    }
}
