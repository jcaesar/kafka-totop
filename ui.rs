use termion::{event::Key, input::TermRead, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{Axis, Cell, Chart, Dataset, GraphType, Row, Table},
    Terminal,
};

use crate::uses::*;

pub(crate) fn run(opts: &Opts) -> Result<()> {
    let mut scraper = Stats::ingesting(scrape::spawn_threads(opts)?, opts.scrape_interval)?;
    let stdout = std::io::stdout().into_raw_mode()?;
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let (usertx, userrx) = mpsc::sync_channel(1000);
    thread::spawn(|| input(usertx));
    let mut maxy = 1.0f64;
    let mut color_assignment = HashMap::new();
    loop {
        scraper.ingest()?;
        if let Some(discard) = Instant::now().checked_sub(opts.draw_interval.mul_f64(1.1)) {
            scraper.discard_before(discard)
        }
            
        terminal.draw(|f| {
            let mut basestats = scraper.basestats().collect::<Vec<_>>();
            basestats.sort_by_key(|s| -s.seen);
            let basestats = basestats;

            let mut colors = LineColor::all_variants();
            let mut topdogs = basestats
                .iter()
                .take(colors.len())
                .map(|s| s.topic)
                .collect::<HashSet<_>>();
            color_assignment.retain(|topic: &String, _| topdogs.contains(topic.as_str()));
            for color in color_assignment.values() {
                colors.remove(color);
            }
            let mut colors = colors.drain();
            for dog in topdogs.drain() {
                // TODO: It's high time for interned topic string names
                color_assignment
                    .entry(dog.to_string())
                    .or_insert_with(|| colors.next().expect("no more dogs than colors"));
            }
            let topic_color = |topic: &str| {
                color_assignment
                    .get(topic)
                    .map(|c| (*c).into())
                    .unwrap_or(Color::Gray)
            };

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(10), Constraint::Length(46)].as_ref())
                .split(f.size());
            let table = Table::new(basestats.iter().map(
                |stats::TopicStats {
                     topic, total, rate, ..
                 }| {
                    Row::new(vec![
                        Cell::from(Span::styled(
                            *topic,
                            Style::default().fg(topic_color(topic)),
                        )),
                        Cell::from(right_align(format_number(*total as f64), 7)),
                        Cell::from(right_align(rate.map(format_number).unwrap_or_default(), 7)),
                    ])
                },
            ))
            .style(Style::default().fg(Color::White))
            .header(Row::new(vec!["Topic", "Total", "Per Sec"]).style(Style::default()))
            .widths(&[
                Constraint::Length(30),
                Constraint::Length(7),
                Constraint::Length(7),
            ])
            .column_spacing(1)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(">");
            f.render_widget(table, chunks[1]);

            let height = f.size().height - 2;
            let width = f.size().width - 7;
            let buckets = width as u32 * 2;
            let now_date = Local::now();
            let (maxv, data) = scraper.rates(opts.draw_interval, buckets);
            if maxy < maxv || maxy > 1.5 * maxv {
                maxy = maxv * 1.25;
            }
            let data = data
                .iter()
                .rev()
                .map(|(topic, padata, _total)| {
                    Dataset::default()
                        .name(*topic)
                        .marker(symbols::Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(topic_color(topic)))
                        .data(&padata)
                })
                .collect();
            let long_time = opts.draw_interval > Duration::from_secs(3600 * 6);
            let date_length = match long_time {
                true => 19,
                false => 8,
            };
            let space = 5;
            let chart = Chart::new(data)
                .hidden_legend_constraints((Constraint::Percentage(0), Constraint::Percentage(0)))
                .x_axis(
                    Axis::default()
                        .style(Style::default().fg(Color::White))
                        .bounds([-opts.draw_interval.as_secs_f64(), 0.0])
                        .labels(
                            once(Span::from(""))
                                .chain((1..=width).step_by(date_length + space).map(|i| {
                                    Span::from(
                                        chrono::Duration::from_std(
                                            opts.draw_interval.mul_f64(1. - i as f64 / width as f64),
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
            f.render_widget(chart, chunks[0]);
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
        NumberPrefix::Standalone(num) => format!("{:.2}", num),
        NumberPrefix::Prefixed(pfx, num) => format!("{:.2}{}", num, pfx),
    }
}

fn right_align(inp: String, len: usize) -> String {
    match len.checked_sub(inp.len()) {
        Some(0) | None => inp,
        Some(fill) => format!("{}{}", " ".repeat(fill), inp),
    }
}

fn input(usertx: mpsc::SyncSender<Result<Key, io::Error>>) -> Result<()> {
    for evt in io::stdin().keys() {
        usertx.send(evt).context("Send fail")?;
    }
    anyhow::bail!("Unexpected input closed");
}

// Bit stupid to duplicate this here, but Color can't be inserted into a hash map
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LineColor {
    Blue,
    Yellow,
    Red,
    Green,
    Magenta,
    Cyan,
    White,
}

impl Into<Color> for LineColor {
    fn into(self) -> Color {
        match self {
            LineColor::Blue => Color::Blue,
            LineColor::Yellow => Color::Yellow,
            LineColor::Red => Color::Red,
            LineColor::Green => Color::Green,
            LineColor::Magenta => Color::Magenta,
            LineColor::Cyan => Color::Cyan,
            LineColor::White => Color::White,
        }
    }
}
impl LineColor {
    pub(crate) fn all_variants() -> HashSet<Self> {
        use LineColor::*;
        [Blue, Yellow, Red, Green, Magenta, Cyan, White]
            .iter()
            .map(Clone::clone)
            .collect()
    }
}
