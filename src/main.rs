use clap::Clap;
use anyhow::{Result, Context};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use rdkafka::{
    ClientConfig as KafkaConfig,
};

/// Are my messages flowing?
#[derive(Clap, Debug)]
#[clap(version = "0.1", author = "Julius Michaelis")]
struct Opts {
    /// Bootstrap broker address
    #[clap(short, long)]
    brokers: String,
    /// Additional kafka producer options
    #[clap(short = 'X', long, parse(try_from_str = parseopts))]
    kafka_options: Vec<(String, String)>,

    /// Polling interval
    #[clap(short, long, default_value = "10 s", parse(try_from_str = parsehuman))]
    interval: Duration,

    /// Polling interval
    #[clap(short, long, default_value = "60 s", parse(try_from_str = parsehuman))]
    timeout: Duration,
//
//    #[clap(subcommand)]
//    mode: Mode
}

//#[derive(Clap, Debug)]
//enum Mode {
//    Export {
//
//    },
//    Show
//}

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

type MetaData = HashMap<String, HashMap<i32, (i64, Instant)>>;

fn main() -> Result<()> {
    let opts = Opts::parse();
    let mut config = KafkaConfig::new();
    config.set("bootstrap.servers", opts.brokers);
    for (k, v) in opts.kafka_options {
        config.set(k, v);
    }
    let client: rdkafka::admin::AdminClient<_> = config.create()
        .context("Failed to construct client")?;
    let client = client.inner();
    //let client: rdkafka::producer::ThreadedProducer<_> = config.create()
    //    .context("Failed to construct client")?;
    //use rdkafka::producer::Producer;
    //let client = client.client();
    let mut next = Instant::now() + opts.interval / 10;
    let mut offsetss = MetaData::new();
    loop {
        println!("\n{}", chrono::Local::now());
        match client.fetch_metadata(None, opts.timeout) {
            Ok(metadata) => {
                for topic in metadata.topics() {
                    let name = topic.name();
                    let offsets = offsetss.entry(name.into()).or_insert_with(HashMap::new);
                    let mut fetch_failed = 0_usize;
                    let mut out_of_sync = 0_usize;
                    let mut agg_rate = 0.0_f64;
                    let mut new = true;
                    for partition in topic.partitions() {
                        let id = partition.id();
                        if partition.isr().len() < partition.replicas().len() {
                            out_of_sync += 1;
                        }
                        match client.fetch_watermarks(name, id, opts.timeout) {
                            Ok((_low, high)) => {
                               let now = Instant::now();
                                offsets.entry(id).and_modify(|(old_high, old_t)| {
                                    agg_rate += ((high - *old_high) as f64) / (now - *old_t).as_secs_f64();
                                    *old_high = high;
                                    *old_t = now;
                                    new = false;
                                }).or_insert((high, now));
                            },
                            Err(_) => fetch_failed += 1,
                        }
                    }
                    match new {
                        true => if fetch_failed != topic.partitions().len() {
                            println!("Topic found: {}", name);
                        },
                        false => {
                            let output = format!("{}: {:.1} msg/s", name, agg_rate);
                            let output = match fetch_failed {
                                0 => output,
                                fetch_failed => format!("{}, unavailable partitions: {}", output, fetch_failed),
                            };
                            let output = match out_of_sync {
                                0 => output,
                                out_of_sync => format!("{}, underreplicated partitions: {}", output, out_of_sync),
                            };
                            println!("{}", output);
                        }
                    }
                }
            }, Err(e) => {
                eprintln!("Failed to query cluster, resting: {:?}", e);
            }
        }
        next += opts.interval;
        let now = Instant::now();
        if let Some(duration) = next.checked_duration_since(now) {
            std::thread::sleep(duration);
        }
    }
}
