use crossterm::{
    event::{self, KeyCode, KeyModifiers},
    terminal::enable_raw_mode,
};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols,
    text::{Span, Spans},
    widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, Wrap},
    Terminal,
};

use crate::uses::*;

pub(crate) fn run(opts: &Opts, mut scraper: Stats) -> Result<()> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut maxy = 1.0f64;
    let mut color_assignment = ColorAssignment::new();
    let mut redraw = true;
    let mut last_draw = Instant::now();
    loop {
        let now = Instant::now();
        redraw |= scraper.ingest()?;
        if let Some(discard) = now.checked_sub(opts.draw_interval.mul_f64(1.1)) {
            scraper.discard_before(discard)
        }
        if now.duration_since(last_draw) > Duration::from_secs(1) {
            redraw |= true;
            last_draw = now;
        }

        if redraw {
            redraw = false;
            terminal.draw(|f| {
                let mut basestats = scraper.basestats().collect::<Vec<_>>();
                basestats.sort_by_key(|s| (s.topic.stat_idx, cmp::Reverse((s.seen, s.total))));
                let basestats = basestats;
                color_assignment.compute(&basestats);

                let content_box;
                if let Some(err) = scraper.metadata_error.as_ref() {
                    let text = vec![Spans::from(vec![Span::styled(
                        err.to_string(),
                        Style::default().fg(Color::Red),
                    )])];
                    let paragraph = Paragraph::new(text)
                        .alignment(Alignment::Center)
                        .wrap(Wrap { trim: true });
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(2), Constraint::Length(1)])
                        .split(f.size());
                    f.render_widget(paragraph, chunks[1]);
                    content_box = chunks[0];
                } else {
                    content_box = f.size();
                }

                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(10), Constraint::Length(46)])
                    .split(content_box);

                f.render_widget(mk_table(&basestats, &color_assignment), chunks[1]);

                let width = chunks[0].width.saturating_sub(9);
                let height = chunks[0].height.saturating_sub(2);
                if cmp::min(width, height) <= 2 {
                    f.render_widget(
                        Paragraph::new(vec![Spans::from("too small"); chunks[0].height as usize])
                            .alignment(Alignment::Center),
                        chunks[0],
                    );
                } else {
                    let bucket_size = opts.draw_interval / (width as u32 * 2);
                    let (now_date, data) = mk_chart_data(
                        bucket_size,
                        &basestats[..cmp::min(basestats.len(), color_assignment.len())],
                        &scraper,
                        &mut maxy,
                    );
                    if data.len() > 0 {
                        let chart = mk_chart(
                            width,
                            height,
                            &data,
                            now_date,
                            opts.draw_interval,
                            &color_assignment,
                            maxy,
                        );
                        f.render_widget(chart, chunks[0]);
                    } else {
                        let text = vec![Spans::from(vec![Span::raw("[no plottable data]")])];
                        let paragraph = Paragraph::new(text)
                            .alignment(Alignment::Center)
                            .block(Block::default())
                            .wrap(Wrap { trim: true });
                        let vsplit_chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Percentage(50),
                                Constraint::Min(1),
                                Constraint::Percentage(50),
                            ])
                            .split(chunks[0]);
                        f.render_widget(paragraph, vsplit_chunks[1])
                    }
                }
            })?;
        }
        if event::poll(Duration::from_millis(100))? {
            redraw = true;
            match event::read() {
                Ok(event::Event::Key(event::KeyEvent { code, modifiers })) => {
                    match (code, modifiers) {
                        (KeyCode::Char('q'), _) => break,
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                        (KeyCode::Char('d'), KeyModifiers::CONTROL) => break,
                        (_, _) => (),
                    }
                }
                Ok(_) => (), // Redraw
                Err(e) => Err(e).context("input error")?,
            }
        }
    }
    Ok(())
}

fn mk_chart<'a>(
    width: u16,
    height: u16,
    data: &'a [(&'a Topic, Vec<(f64, f64)>)],
    now_date: DateTime<Local>,
    draw_interval: Duration,
    color_assignment: &ColorAssignment,
    maxy: f64,
) -> Chart<'a> {
    let data = data
        .iter()
        .map(|(topic, data)| {
            Dataset::default()
                .name(topic.name.to_owned())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color_assignment.get(topic)))
                .data(data)
        })
        .collect();
    let long_time = draw_interval > Duration::from_secs(3600 * 6);
    let date_length = match long_time {
        true => 19,
        false => 8,
    };
    let space = 5;
    let maxl = cmp::max(height / 10, 1);
    let chart = Chart::new(data)
        .hidden_legend_constraints((Constraint::Percentage(0), Constraint::Percentage(0)))
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::White))
                .bounds([-draw_interval.as_secs_f64(), 0.0])
                .labels(
                    once(Span::from(""))
                        .chain((1..=width).step_by(date_length + space).map(|i| {
                            Span::from(
                                chrono::Duration::from_std(
                                    draw_interval.mul_f64(1. - i as f64 / width as f64),
                                )
                                .map(|dur: chrono::Duration| format_time(now_date - dur, long_time))
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
                    (0..=maxl)
                        .map(|p| Span::from(format_number(p as f64 / maxl as f64 * maxy)))
                        .collect(),
                ),
        );
    chart
}

fn mk_chart_data<'a>(
    bucket_size: Duration,
    basestats: &'a [stats::TopicStats],
    scraper: &Stats,
    maxy: &mut f64,
) -> (DateTime<Local>, Vec<(&'a Topic, Vec<(f64, f64)>)>) {
    let now_date = Local::now();
    let now = Instant::now();
    let data = basestats
        .iter()
        .filter(|t| t.seen > 0)
        .filter_map(|stats::TopicStats { topic, .. }| {
            Some((topic, scraper.rates(topic, now, bucket_size)?))
        })
        .collect::<Vec<_>>();
    let maxv = data
        .iter()
        .flat_map(|(_, data)| data.iter().map(|(_, v)| *v))
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(a.is_nan().cmp(&b.is_nan())))
        .unwrap_or(1f64);
    if *maxy < maxv || *maxy > 1.5 * maxv {
        *maxy = maxv * 1.25;
    }
    (now_date, data)
}

fn mk_table<'a>(
    basestats: &'a Vec<stats::TopicStats>,
    color_assignment: &ColorAssignment,
) -> Table<'a> {
    Table::new(basestats.iter().map(
        |stats::TopicStats {
             topic, total, rate, ..
         }| {
            Row::new(vec![
                Cell::from(Span::styled(
                    &topic.name,
                    Style::default().fg(color_assignment.get(topic)),
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
    .highlight_symbol(">")
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
