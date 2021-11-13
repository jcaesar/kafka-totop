use crate::uses::*;

pub(crate) fn run(opts: &Opts) -> Result<()> {
    let mut scraper = Stats::ingesting(scrape::spawn_threads(opts)?)?;
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
