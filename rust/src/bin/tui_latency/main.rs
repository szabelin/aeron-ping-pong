//! Interactive TUI for live ping-pong latency measurement.
//!
//! This TUI is its own ping sender:
//! - Publishes market data on stream 1001 (ping)
//! - Receives echoes on stream 1002 (pong)
//! - Measures RTT using Rust's Instant::now()
//! - Displays live histogram, percentiles, sparkline
//! - Interactive controls: fragment limit, send rate, pause/resume

mod render;
mod state;
mod worker;

use crossbeam_channel::{bounded, Receiver, Sender};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use state::{
    LatencySample, SharedParams, TuiState, FRAGMENT_PRESETS, SPARKLINE_HISTORY, TICKER_SIZE,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let params = Arc::new(SharedParams::new());
    let params_worker = Arc::clone(&params);

    let (sample_tx, sample_rx): (Sender<LatencySample>, Receiver<LatencySample>) = bounded(4096);

    let worker = std::thread::Builder::new()
        .name("ping-pong".into())
        .spawn(move || worker::worker_thread(params_worker, sample_tx))?;

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut tui = TuiState::new();
    let tick_rate = Duration::from_millis(33);

    while params.running.load(Ordering::Relaxed) {
        // Drain samples -- record RTT in histogram + add to ticker
        while let Ok(sample) = sample_rx.try_recv() {
            tui.record(sample.rtt_ns);
            tui.ticker.push(sample);
            if tui.ticker.len() > TICKER_SIZE {
                tui.ticker.remove(0);
            }
        }

        // Update rate every second
        if tui.last_tick.elapsed() >= Duration::from_secs(1) {
            let total = params.total_received.load(Ordering::Relaxed);
            let rate = total - tui.last_total;
            tui.current_rate = rate;
            if rate > tui.peak_rate {
                tui.peak_rate = rate;
            }
            tui.last_total = total;
            tui.last_tick = Instant::now();

            let p50 = if !tui.histogram.is_empty() {
                tui.histogram.value_at_percentile(50.0)
            } else {
                0
            };
            tui.p50_history.remove(0);
            tui.p50_history.push(p50);
        }

        terminal.draw(|frame| render::render(frame, &tui, &params))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        params.running.store(false, Ordering::SeqCst);
                    }
                    KeyCode::Char('1') => {
                        tui.fragment_idx = 0;
                        params.fragment_limit.store(FRAGMENT_PRESETS[0], Ordering::Relaxed);
                    }
                    KeyCode::Char('2') => {
                        tui.fragment_idx = 1;
                        params.fragment_limit.store(FRAGMENT_PRESETS[1], Ordering::Relaxed);
                    }
                    KeyCode::Char('3') => {
                        tui.fragment_idx = 2;
                        params.fragment_limit.store(FRAGMENT_PRESETS[2], Ordering::Relaxed);
                    }
                    KeyCode::Char('4') => {
                        tui.fragment_idx = 3;
                        params.fragment_limit.store(FRAGMENT_PRESETS[3], Ordering::Relaxed);
                    }
                    KeyCode::Char('5') => {
                        tui.fragment_idx = 4;
                        params.fragment_limit.store(FRAGMENT_PRESETS[4], Ordering::Relaxed);
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        let was = params.paused.load(Ordering::Relaxed);
                        params.paused.store(!was, Ordering::SeqCst);
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        tui.histogram.reset();
                        tui.bucket_counts = [0; 7];
                        tui.p50_history = vec![0; SPARKLINE_HISTORY];
                        tui.p99_history = vec![0; SPARKLINE_HISTORY];
                        tui.peak_rate = 0;
                        tui.start_time = Instant::now();
                        tui.last_tick = Instant::now();
                        tui.last_total = params.total_received.load(Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    params.running.store(false, Ordering::SeqCst);
    let _ = worker.join();

    println!("\n=== TUI Latency shutdown ===");
    println!("Total round-trips: {}", params.total_received.load(Ordering::Relaxed));

    Ok(())
}
