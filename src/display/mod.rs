use crate::metrics::{MetricsSnapshot, SharedMetrics};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Sparkline},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Real-time stats display using TUI
pub struct StatsDisplay {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    test_name: String,
    target: String,
    concurrency: u32,
    duration: Option<Duration>,
    start_time: Instant,
    rps_history: Vec<u64>,
    latency_history: Vec<u64>,
}

impl StatsDisplay {
    pub fn new(
        metrics: SharedMetrics,
        test_name: &str,
        target: &str,
        concurrency: u32,
        duration: Option<Duration>,
    ) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            metrics,
            running: Arc::new(AtomicBool::new(true)),
            test_name: test_name.to_string(),
            target: target.to_string(),
            concurrency,
            duration,
            start_time: Instant::now(),
            rps_history: Vec::with_capacity(60),
            latency_history: Vec::with_capacity(60),
        })
    }

    pub fn running_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Run the display loop
    pub async fn run(&mut self, update_interval: Duration) -> io::Result<()> {
        let mut last_count = 0u64;
        let mut last_time = Instant::now();

        loop {
            // Check for keyboard input
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                self.running.store(false, Ordering::SeqCst);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            // Check if test duration exceeded
            if let Some(duration) = self.duration {
                if self.start_time.elapsed() >= duration {
                    self.running.store(false, Ordering::SeqCst);
                    break;
                }
            }

            // Get current metrics
            let snapshot = self.metrics.full_snapshot().await;

            // Calculate instantaneous RPS
            let now = Instant::now();
            let time_delta = now.duration_since(last_time).as_secs_f64();
            let count_delta = snapshot.counters.total.saturating_sub(last_count);
            let instant_rps = if time_delta > 0.0 {
                (count_delta as f64 / time_delta) as u64
            } else {
                0
            };

            // Update history
            self.rps_history.push(instant_rps);
            if self.rps_history.len() > 60 {
                self.rps_history.remove(0);
            }

            self.latency_history
                .push(snapshot.latency.p99.as_micros() as u64 / 1000); // ms
            if self.latency_history.len() > 60 {
                self.latency_history.remove(0);
            }

            last_count = snapshot.counters.total;
            last_time = now;

            // Draw UI - capture all data needed first
            let draw_data = DrawData {
                test_name: self.test_name.clone(),
                target: self.target.clone(),
                concurrency: self.concurrency,
                duration: self.duration,
                start_time: self.start_time,
                rps_history: self.rps_history.clone(),
                latency_history: self.latency_history.clone(),
                snapshot,
                instant_rps,
            };

            self.terminal.draw(|f| {
                draw_ui(f, &draw_data);
            })?;

            tokio::time::sleep(update_interval).await;
        }

        Ok(())
    }
}

struct DrawData {
    test_name: String,
    target: String,
    concurrency: u32,
    duration: Option<Duration>,
    start_time: Instant,
    rps_history: Vec<u64>,
    latency_history: Vec<u64>,
    snapshot: MetricsSnapshot,
    instant_rps: u64,
}

fn draw_ui(f: &mut Frame, data: &DrawData) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Progress
            Constraint::Length(8),  // Stats
            Constraint::Length(5),  // RPS Sparkline
            Constraint::Length(5),  // Latency Sparkline
            Constraint::Min(3),     // Status codes
        ])
        .split(f.area());

    // Header
    draw_header(f, chunks[0], data);

    // Progress bar (if duration-based)
    draw_progress(f, chunks[1], data);

    // Main stats
    draw_stats(f, chunks[2], data);

    // RPS sparkline
    draw_rps_sparkline(f, chunks[3], data);

    // Latency sparkline
    draw_latency_sparkline(f, chunks[4], data);

    // Status codes
    draw_status_codes(f, chunks[5], data);
}

fn draw_header(f: &mut Frame, area: Rect, data: &DrawData) {
    let elapsed = data.start_time.elapsed();
    let header_text = format!(
        "TrafficCop Bench │ {} │ {} │ {} workers │ Elapsed: {:.1}s │ Press 'q' to quit",
        data.test_name,
        data.target,
        data.concurrency,
        elapsed.as_secs_f64()
    );

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title("Load Test"));

    f.render_widget(header, area);
}

fn draw_progress(f: &mut Frame, area: Rect, data: &DrawData) {
    if let Some(duration) = data.duration {
        let elapsed = data.start_time.elapsed();
        let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);
        let remaining = duration.saturating_sub(elapsed);

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Progress"))
            .gauge_style(Style::default().fg(Color::Green))
            .percent((progress * 100.0) as u16)
            .label(format!(
                "{:.1}% ({:.0}s remaining)",
                progress * 100.0,
                remaining.as_secs_f64()
            ));

        f.render_widget(gauge, area);
    } else {
        let text = Paragraph::new("Running until stopped (press 'q' to stop)")
            .block(Block::default().borders(Borders::ALL).title("Progress"));
        f.render_widget(text, area);
    }
}

fn draw_stats(f: &mut Frame, area: Rect, data: &DrawData) {
    let snapshot = &data.snapshot;
    let instant_rps = data.instant_rps;

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(area);

    // Throughput stats
    let throughput_text = vec![
        Line::from(vec![
            Span::raw("Current:  "),
            Span::styled(
                format!("{} req/s", instant_rps),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("Average:  "),
            Span::styled(
                format!("{:.0} req/s", snapshot.requests_per_second),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(format!("Total:    {} requests", snapshot.counters.total)),
        Line::from(format!("Success:  {}", snapshot.counters.success)),
        Line::from(vec![
            Span::raw("Errors:   "),
            Span::styled(
                format!("{}", snapshot.counters.errors + snapshot.counters.timeouts),
                Style::default().fg(if snapshot.counters.errors > 0 {
                    Color::Red
                } else {
                    Color::Green
                }),
            ),
        ]),
    ];

    let throughput = Paragraph::new(throughput_text)
        .block(Block::default().borders(Borders::ALL).title("Throughput"));
    f.render_widget(throughput, chunks[0]);

    // Latency stats
    let latency_text = vec![
        Line::from(format!(
            "p50:   {}",
            format_duration(snapshot.latency.p50)
        )),
        Line::from(format!(
            "p90:   {}",
            format_duration(snapshot.latency.p90)
        )),
        Line::from(format!(
            "p95:   {}",
            format_duration(snapshot.latency.p95)
        )),
        Line::from(vec![
            Span::raw("p99:   "),
            Span::styled(
                format_duration(snapshot.latency.p99),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::raw("p99.9: "),
            Span::styled(
                format_duration(snapshot.latency.p999),
                Style::default().fg(Color::Red),
            ),
        ]),
    ];

    let latency = Paragraph::new(latency_text)
        .block(Block::default().borders(Borders::ALL).title("Latency"));
    f.render_widget(latency, chunks[1]);

    // Data transfer
    let data_text = vec![
        Line::from(format!("Sent:     {}", format_bytes(snapshot.bytes.sent))),
        Line::from(format!(
            "Received: {}",
            format_bytes(snapshot.bytes.received)
        )),
        Line::from(format!(
            "Timeouts: {}",
            snapshot.counters.timeouts
        )),
        Line::from(format!(
            "Conn Err: {}",
            snapshot.counters.connection_errors
        )),
        Line::from(vec![
            Span::raw("Success:  "),
            Span::styled(
                format!("{:.1}%", snapshot.counters.success_rate() * 100.0),
                Style::default().fg(if snapshot.counters.success_rate() > 0.99 {
                    Color::Green
                } else if snapshot.counters.success_rate() > 0.95 {
                    Color::Yellow
                } else {
                    Color::Red
                }),
            ),
        ]),
    ];

    let data_widget = Paragraph::new(data_text)
        .block(Block::default().borders(Borders::ALL).title("Data"));
    f.render_widget(data_widget, chunks[2]);
}

fn draw_rps_sparkline(f: &mut Frame, area: Rect, data: &DrawData) {
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Requests/sec (last 60 samples)"),
        )
        .data(&data.rps_history)
        .style(Style::default().fg(Color::Green));

    f.render_widget(sparkline, area);
}

fn draw_latency_sparkline(f: &mut Frame, area: Rect, data: &DrawData) {
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("p99 Latency (ms, last 60 samples)"),
        )
        .data(&data.latency_history)
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(sparkline, area);
}

fn draw_status_codes(f: &mut Frame, area: Rect, data: &DrawData) {
    let snapshot = &data.snapshot;
    let mut codes: Vec<_> = snapshot.status_codes.iter().collect();
    codes.sort_by_key(|(code, _)| *code);

    let lines: Vec<Line> = codes
        .iter()
        .map(|(code, count)| {
            let color = if **code >= 200 && **code < 300 {
                Color::Green
            } else if **code >= 400 && **code < 500 {
                Color::Yellow
            } else if **code >= 500 {
                Color::Red
            } else {
                Color::Gray
            };

            Line::from(vec![
                Span::styled(format!("{}: ", code), Style::default().fg(color)),
                Span::raw(format!("{}", count)),
            ])
        })
        .collect();

    let status = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Status Codes"));
    f.render_widget(status, area);
}

impl Drop for StatsDisplay {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

fn format_duration(d: Duration) -> String {
    let us = d.as_micros();
    if us >= 1_000_000 {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{:.2}ms", us as f64 / 1_000.0)
    } else {
        format!("{}µs", us)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Simple progress printer for non-TUI mode
pub struct SimpleProgress {
    metrics: SharedMetrics,
    running: Arc<AtomicBool>,
    interval: Duration,
}

impl SimpleProgress {
    pub fn new(metrics: SharedMetrics, interval: Duration) -> Self {
        Self {
            metrics,
            running: Arc::new(AtomicBool::new(true)),
            interval,
        }
    }

    pub fn running_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    pub async fn run(&self) {
        let mut last_count = 0u64;
        let mut last_time = Instant::now();

        while self.running.load(Ordering::SeqCst) {
            tokio::time::sleep(self.interval).await;

            let snapshot = self.metrics.full_snapshot().await;

            // Calculate instantaneous RPS
            let now = Instant::now();
            let time_delta = now.duration_since(last_time).as_secs_f64();
            let count_delta = snapshot.counters.total.saturating_sub(last_count);
            let instant_rps = if time_delta > 0.0 {
                (count_delta as f64 / time_delta) as u64
            } else {
                0
            };

            last_count = snapshot.counters.total;
            last_time = now;

            println!(
                "[{:.1}s] {} req | {} req/s (avg {:.0}) | p99: {} | errors: {}",
                snapshot.duration.as_secs_f64(),
                snapshot.counters.total,
                instant_rps,
                snapshot.requests_per_second,
                format_duration(snapshot.latency.p99),
                snapshot.counters.errors + snapshot.counters.timeouts
            );
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
