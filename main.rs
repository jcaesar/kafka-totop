pub mod colors;
pub mod scrape;
pub mod stats;
pub mod ui;
pub mod uses;

use crossterm::terminal::disable_raw_mode;
use uses::*;

/// Are my messages flowing?
#[derive(StructOpt, Debug)]
#[structopt(version = "0.1", author = "Julius Michaelis")]
pub struct Opts {
    /// Bootstrap broker address
    #[structopt(short, long)]
    brokers: String,
    /// Additional kafka client options
    #[structopt(short = "X", long, parse(try_from_str = parseopts))]
    kafka_options: Vec<(String, String)>,

    /// Length of history to draw as graph
    #[structopt(short, long, default_value = "15 min", parse(try_from_str = parsehuman))]
    draw_interval: Duration,

    /// Polling interval
    #[structopt(short, long, default_value = "10 s", parse(try_from_str = parsehuman))]
    scrape_interval: Duration,
    /// Metadata retrieval timeout
    #[structopt(short = "T", long, default_value = "5 s", parse(try_from_str = parsehuman))]
    scrape_timeout: Duration,
}

fn parseopts(arg: &str) -> Result<(String, String)> {
    let mut split = arg.splitn(2, '=');
    if let (Some(k), Some(v), None) = (split.next(), split.next(), split.next()) {
        Ok((k.to_owned(), v.to_owned()))
    } else {
        anyhow::bail!("Expected a parameter of form config.key=value, got {}", arg);
    }
}

fn parsehuman(arg: &str) -> Result<Duration> {
    Ok(arg
        .parse::<humantime::Duration>()
        .context(format!("Not a parsaeble time: {}", arg))?
        .into())
}

fn main() -> Result<()> {
    let opts = Opts::from_args();
    let scrape = scrape::spawn_threads(&opts);
    let stats = Stats::ingesting(scrape?, opts.scrape_interval)?;
    std::panic::set_hook(Box::new(move |info| {
        disable_raw_mode().ok();
        better_panic::Settings::new().create_panic_handler()(info);
    }));
    let res = ui::run(&opts, stats);
    let dis = disable_raw_mode();
    res?;
    dis?;
    Ok(())
}
