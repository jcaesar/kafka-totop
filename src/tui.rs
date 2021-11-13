use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, Timelike};
use itertools::Itertools;
use number_prefix::NumberPrefix;
use rand::Rng;
use std::{
    collections::{hash_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    iter::once,
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use termion::{event::Key, input::TermRead, raw::IntoRawMode, screen::AlternateScreen};
use ::tui::{
    backend::TermionBackend,
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{Axis, Chart, Dataset, GraphType},
    Terminal,
};

use crate::*;

#[derive(Default)]
struct State {
    bad_brokers: Mutex<HashSet<i32>>,
    query_interval: Duration,
}

struct Scrapes {
    data: HashMap<String, HashMap<i32, BTreeMap<Instant, i64>>>,
    offrx: Receiver<Message>,
}

impl Scrapes {
    fn scrape(opts: &Opts) -> Result<Self, anyhow::Error> {
        let state = Arc::new(State {
            query_interval: opts.interval,
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
        Ok(Self {
            data: HashMap::new(),
            offrx,
        })
    }
    fn ingest(&mut self) -> Result<()> {
        loop {
            match self.offrx.try_recv() {
                Ok(Message::PartitionOffsets {
                    topic,
                    partition,
                    offset,
                    now,
                }) => {
                    self.data
                        .entry(topic)
                        .or_insert_with(HashMap::new)
                        .entry(partition)
                        .or_insert_with(BTreeMap::new)
                        .insert(now, offset);
                }
                Ok(msg) => anyhow::bail!("TODO: handle {:?}", msg),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    // TODO: poll thread exit for an error for a second or so
                    anyhow::bail!("Metadata gatherer bailed");
                }
            }
        }
        // TODO: Discard old values
    }
    fn rates(&self, interval: Duration, buckets: u32) -> (f64, Vec<(&str, Vec<(f64, f64)>, i64)>) {
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
                for (_, polls) in padata.iter() {
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
}

pub(crate) fn run(opts: &Opts) -> Result<()> {
    let mut scraper = Scrapes::scrape(opts)?;
    let stdout = std::io::stdout().into_raw_mode()?;
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let (usertx, userrx) = mpsc::sync_channel(1000);
    thread::spawn(|| input(usertx));
    let mut maxy = 1.0f64;
    loop {
        scraper.ingest()?;
        terminal.draw(|f| {
            let interval = Duration::from_secs(900); // TODO: CLI option
            let height = f.size().height - 2;
            let width = f.size().width - 7;
            let buckets = width as u32 * 2;
            let now_date = Local::now();
            let (maxv, data) = scraper.rates(interval, buckets);
            if maxy < maxv || maxy > 1.5 * maxv {
                maxy = maxv * 1.25;
            }
            // TODO: color most active
            let mut colors = vec![
                Color::White,
                Color::Blue,
                Color::Yellow,
                Color::Red,
                Color::Green,
                Color::Magenta,
                Color::Cyan,
            ];
            let data = data
                .iter()
                .map(|(topic, padata, _total)| {
                    Dataset::default()
                        .name(*topic)
                        .marker(symbols::Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(colors.pop().unwrap_or(Color::Gray)))
                        .data(&padata)
                })
                .collect();
            let long_time = interval > Duration::from_secs(3600 * 6);
            let date_length = match long_time {
                true => 19,
                false => 8,
            };
            let space = 5;
            let chart = Chart::new(data)
                .x_axis(
                    Axis::default()
                        .style(Style::default().fg(Color::White))
                        .bounds([-interval.as_secs_f64(), 0.0])
                        .labels(
                            once(Span::from(""))
                                .chain((1..=width).step_by(date_length + space).map(|i| {
                                    Span::from(
                                        chrono::Duration::from_std(
                                            interval.mul_f64(1. - i as f64 / width as f64),
                                        )
                                        .map(|dur: chrono::Duration| {
                                            format_time(now_date - dur, long_time)
                                        })
                                        .unwrap_or("X".repeat(date_length)),
                                    )
                                }))
                                .collect(),
                        ),
                )
                .y_axis(
                    Axis::default()
                        .title(Span::styled("Msgs / s", Style::default().fg(Color::Red)))
                        .style(Style::default().fg(Color::White))
                        .bounds([0.0, maxy])
                        .labels(
                            (0..=height / 10)
                                .map(|p| {
                                    Span::from(format_number(
                                        p as f64 / (height / 10) as f64 * maxy,
                                    ))
                                })
                                .collect(),
                        ),
                );
            f.render_widget(chart, f.size());
        })?;
        match userrx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(Key::Char('q'))) => break,
            Ok(Ok(_)) => (), // TODO: allow scaling max
            Ok(Err(e)) => Err(e).context("Stdin read error")?,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("stdin watcher died"), // TODO: poll thread join
        }
    }
    Ok(())
}

fn format_time(time: DateTime<Local>, long: bool) -> String {
    match long {
        false => format!(
            "{:02}:{:02}:{:02}",
            time.hour(),
            time.minute(),
            time.second()
        ),
        true => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second()
        ),
    }
}

fn format_number(num: f64) -> String {
    match NumberPrefix::decimal(num) {
        NumberPrefix::Standalone(num) => format!("{:.0}", num),
        NumberPrefix::Prefixed(pfx, num) => format!("{:.2}{}", num, pfx),
    }
}

fn input(usertx: mpsc::SyncSender<Result<Key, io::Error>>) -> Result<()> {
    for evt in io::stdin().keys() {
        usertx.send(evt).context("Send fail")?;
    }
    anyhow::bail!("Unexpected input closed");
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

#[derive(Debug)]
enum Message {
    MetadataQueryFail,
    BrokerQueryFail(String),
    PartitionOffsets {
        now: Instant,
        topic: String, // TODO: intern
        partition: i32,
        offset: i64,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QueryTask {
    when: Instant,
    topic: String, // TODO: intern
    partition: i32,
    leader: i32,
}

fn query_offsets(state: Arc<State>, tx: mpsc::SyncSender<Message>, client: Client) -> Result<()> {
    let mut next = Instant::now();
    let client = client.inner();
    loop {
        next += state.query_interval;

        let mut query_tasks = BTreeSet::new(); // Better structures exist for scheduling.
        // TODO: Timeout probably shouldn't be the entire query interval...
        match client.fetch_metadata(None, state.query_interval) {
            Ok(metadata) => {
                let now = Instant::now();
                for topic in metadata.topics() {
                    let partitions = topic.partitions();
                    let shift = state.query_interval / partitions.len() as u32;
                    let offset = now + shift.mul_f64(rand_seeder::SipHasher::from(topic.name()).into_rng().gen());
                    for (i, partition) in partitions.iter().enumerate() {
                        query_tasks.insert(QueryTask {
                            when: offset + shift * i as u32,
                            topic: topic.name().into(),
                            partition: partition.id(),
                            leader: partition.leader(),
                        });
                    }
                }
            }
            Err(_) => (), // TODO?
        }

        let mut bads = state.bad_brokers.lock().expect("poisoned").clone();
        for QueryTask {
            when,
            topic,
            partition,
            leader,
        } in query_tasks.range(..)
        {
            if bads.contains(leader) {
                continue;
            }
            if let Some(sleep) = when.checked_duration_since(Instant::now()) {
                thread::sleep(sleep);
            }
            match client.fetch_watermarks(topic, *partition, state.query_interval) {
                Ok((_low, high)) => tx.send(Message::PartitionOffsets {
                    now: Instant::now(),
                    topic: topic.into(),
                    partition: *partition,
                    offset: high,
                })?,

                Err(_) => {
                    bads.insert(*leader);
                }
            };
        }

        let now = Instant::now();
        if let Some(sleep) = next.checked_duration_since(now) {
            thread::sleep(sleep);
        } else {
            next = now;
        }
    }
}
