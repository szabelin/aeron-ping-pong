//! TUI rendering functions.
//!
//! All ratatui widget construction and frame rendering lives here.
//! Each panel (histogram, sparkline, ticker, percentiles, controls, stats)
//! has its own function, composed together by the top-level [`render`] function.

use aeron_ping_pong::format_count;
use crate::state::{SharedParams, TuiState, FRAGMENT_PRESETS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Frame,
};
use std::sync::atomic::Ordering;

// =============================================================================
// HELPERS
// =============================================================================

fn format_latency(ns: u64) -> String {
    if ns < 1_000 {
        format!("{}ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.1}\u{03bc}s", ns as f64 / 1_000.0)
    } else {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    }
}

fn latency_color(ns: u64) -> Color {
    if ns < 1_000 {
        Color::Green
    } else if ns < 10_000 {
        Color::Yellow
    } else if ns < 100_000 {
        Color::Rgb(255, 165, 0)
    } else {
        Color::Red
    }
}

fn pct_line(label: &str, ns: u64) -> Line<'static> {
    let text = format_latency(ns);
    let color = latency_color(ns);
    Line::from(vec![
        Span::styled(label.to_string(), Style::default().fg(Color::White)),
        Span::styled(
            text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}


// =============================================================================
// RENDERING
// =============================================================================

pub fn render(frame: &mut Frame, tui: &TuiState, params: &SharedParams) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(frame.area());

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Min(6),
        ])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Min(4),
        ])
        .split(main_chunks[1]);

    render_histogram_bars(frame, left_chunks[0], tui);
    render_sparkline(frame, left_chunks[1], tui);
    render_ticker(frame, left_chunks[2], tui);
    render_percentiles(frame, right_chunks[0], tui);
    render_controls(frame, right_chunks[1], tui, params);
    render_stats(frame, right_chunks[2], tui, params);
}

fn render_histogram_bars(frame: &mut Frame, area: Rect, tui: &TuiState) {
    let block = Block::default()
        .title(" Latency Distribution (HDR Histogram) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let total: u64 = tui.bucket_counts.iter().sum();
    if total == 0 {
        let paragraph = Paragraph::new(" Waiting for data... (ensure Rust pong is running)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    // Horizontal bar chart using text -- looks much better for latency distributions
    let max_count = *tui.bucket_counts.iter().max().unwrap_or(&1);
    let bar_width = (area.width as usize).saturating_sub(24); // space for label + count

    let colors = [
        Color::Green,
        Color::Green,
        Color::Yellow,
        Color::Yellow,
        Color::Rgb(255, 165, 0),
        Color::Red,
        Color::Red,
    ];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![]));

    for (i, (label, &count)) in tui
        .bucket_labels
        .iter()
        .zip(tui.bucket_counts.iter())
        .enumerate()
    {
        let pct = if total > 0 {
            count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let bar_len = if max_count > 0 {
            (count as f64 / max_count as f64 * bar_width as f64) as usize
        } else {
            0
        };
        let bar: String = "\u{2588}".repeat(bar_len);
        let empty: String = "\u{2591}".repeat(bar_width.saturating_sub(bar_len));

        lines.push(Line::from(vec![
            Span::styled(format!(" {:>7} ", label), Style::default().fg(Color::White)),
            Span::styled(bar, Style::default().fg(colors[i])),
            Span::styled(empty, Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" {:>5.1}%", pct),
                Style::default().fg(colors[i]).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_sparkline(frame: &mut Frame, area: Rect, tui: &TuiState) {
    let block = Block::default()
        .title(" p50 Latency Over Time (ns) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let sparkline = Sparkline::default()
        .block(block)
        .data(&tui.p50_history)
        .style(Style::default().fg(Color::Green));

    frame.render_widget(sparkline, area);
}

fn render_ticker(frame: &mut Frame, area: Rect, tui: &TuiState) {
    let block = Block::default()
        .title(" Recent Trades (sampled) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        format!(
            " {:>10} {:>8}  {:>4}  {:>10}  {:>8}  {:>8}",
            "Seq", "Symbol", "Side", "Price", "Qty", "RTT"
        ),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )]));

    for sample in tui.ticker.iter().rev() {
        let symbol = std::str::from_utf8(&sample.symbol).unwrap_or("???").trim();
        let (side_str, side_color, arrow) = if sample.is_buy {
            ("BUY ", Color::Green, "\u{25b2}")
        } else {
            ("SELL", Color::Red, "\u{25bc}")
        };
        let rtt_str = format_latency(sample.rtt_ns);
        let rtt_color = latency_color(sample.rtt_ns);

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:>10} ", sample.seq),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("{:>8}", symbol), Style::default().fg(Color::White)),
            Span::styled(
                format!("  {}{} ", side_str, arrow),
                Style::default().fg(side_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>10.2}", sample.price),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("  {:>8.4}", sample.qty),
                Style::default().fg(Color::White),
            ),
            Span::styled(format!("  {:>8}", rtt_str), Style::default().fg(rtt_color)),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_percentiles(frame: &mut Frame, area: Rect, tui: &TuiState) {
    let block = Block::default()
        .title(" Latency Percentiles ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let count = tui.histogram.len();
    if count == 0 {
        let paragraph = Paragraph::new(" Waiting for latency data...")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let p50 = tui.histogram.value_at_percentile(50.0);
    let p90 = tui.histogram.value_at_percentile(90.0);
    let p99 = tui.histogram.value_at_percentile(99.0);
    let p999 = tui.histogram.value_at_percentile(99.9);
    let p9999 = tui.histogram.value_at_percentile(99.99);
    let min = tui.histogram.min();
    let max = tui.histogram.max();
    let mean = tui.histogram.mean();

    let lines = vec![
        Line::from(vec![]),
        pct_line("  Min:    ", min),
        pct_line("  p50:    ", p50),
        pct_line("  p90:    ", p90),
        pct_line("  p99:    ", p99),
        pct_line("  p99.9:  ", p999),
        pct_line("  p99.99: ", p9999),
        pct_line("  Max:    ", max),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("  Mean:   ", Style::default().fg(Color::White)),
            Span::styled(
                format_latency(mean as u64),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Count:  ", Style::default().fg(Color::White)),
            Span::styled(format_count(count), Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_controls(frame: &mut Frame, area: Rect, tui: &TuiState, params: &SharedParams) {
    let block = Block::default()
        .title(" Controls ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paused = params.paused.load(Ordering::Relaxed);
    let status_style = if paused {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    };
    let status_text = if paused {
        "\u{25a0} PAUSED "
    } else {
        "\u{25cf} RUNNING"
    };

    let frag_display: Vec<Span> = FRAGMENT_PRESETS
        .iter()
        .enumerate()
        .flat_map(|(i, &v)| {
            let is_sel = i == tui.fragment_idx;
            let style = if is_sel {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            vec![
                Span::raw(if is_sel { "[" } else { " " }),
                Span::styled(format!("{}", v), style),
                Span::raw(if is_sel { "]" } else { " " }),
            ]
        })
        .collect();

    let lines = vec![
        Line::from(vec![
            Span::styled(" [P] ", Style::default().fg(Color::Cyan)),
            Span::raw("Status:    "),
            Span::styled(status_text, status_style),
        ]),
        Line::from(vec![]),
        Line::from(
            std::iter::once(Span::styled(" [1-5] ", Style::default().fg(Color::Cyan)))
                .chain(std::iter::once(Span::raw("Fragment: ")))
                .chain(frag_display)
                .collect::<Vec<_>>(),
        ),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled(" Mode: ", Style::default().fg(Color::White)),
            Span::styled(
                "send-one \u{2192} wait-pong \u{2192} measure \u{2192} repeat",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("   [R] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Reset  "),
            Span::styled("   [Q] ", Style::default().fg(Color::DarkGray)),
            Span::raw("Quit"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_stats(frame: &mut Frame, area: Rect, tui: &TuiState, params: &SharedParams) {
    let block = Block::default()
        .title(" Stats ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let total = params.total_received.load(Ordering::Relaxed);
    let elapsed = tui.start_time.elapsed();
    let uptime = format!("{}m {:02}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60);

    let rate_color = if tui.current_rate > 100_000 {
        Color::Green
    } else if tui.current_rate > 10_000 {
        Color::Yellow
    } else {
        Color::White
    };

    let timeouts = params.timeouts.load(Ordering::Relaxed);
    let drops = params.dropped_samples.load(Ordering::Relaxed);

    let lines = vec![
        Line::from(vec![
            Span::styled(" Rate:   ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} msgs/sec", format_count(tui.current_rate)),
                Style::default().fg(rate_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  Total: {}  Up: {}", format_count(total), uptime),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Timeouts: ", Style::default().fg(Color::White)),
            Span::styled(
                format_count(timeouts),
                Style::default().fg(if timeouts > 0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
            Span::styled("  Dropped: ", Style::default().fg(Color::White)),
            Span::styled(
                format_count(drops),
                Style::default().fg(if drops > 0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
