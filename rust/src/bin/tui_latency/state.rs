//! Shared state and types for the TUI latency binary.
//!
//! Contains the lock-free atomic state shared between the worker thread and the
//! TUI rendering loop, the latency sample type sent over the channel, and the
//! TUI-local display state (histogram, ticker, sparkline history).

use hdrhistogram::Histogram;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::time::Instant;

// =============================================================================
// SHARED STATE (atomics for lock-free worker <-> TUI communication)
// =============================================================================

pub struct SharedParams {
    pub running: AtomicBool,
    pub paused: AtomicBool,
    pub fragment_limit: AtomicU32,
    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub timeouts: AtomicU64,
    pub dropped_samples: AtomicU64,
}

impl SharedParams {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            fragment_limit: AtomicU32::new(10),
            total_sent: AtomicU64::new(0),
            total_received: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            dropped_samples: AtomicU64::new(0),
        }
    }
}

// =============================================================================
// LATENCY SAMPLE (sent worker -> TUI)
// =============================================================================

pub struct LatencySample {
    pub rtt_ns: u64,
    pub symbol: [u8; 8],
    pub price: f64,
    pub qty: f64,
    pub is_buy: bool,
    pub seq: u64,
}

// =============================================================================
// TUI STATE
// =============================================================================

pub const FRAGMENT_PRESETS: [u32; 5] = [1, 10, 50, 256, 1024];
pub const SPARKLINE_HISTORY: usize = 120;
pub const TICKER_SIZE: usize = 10;

pub struct TuiState {
    pub histogram: Histogram<u64>,
    pub ticker: Vec<LatencySample>,
    pub p50_history: Vec<u64>,
    pub p99_history: Vec<u64>,
    pub last_tick: Instant,
    pub start_time: Instant,
    pub current_rate: u64,
    pub last_total: u64,
    pub peak_rate: u64,
    pub bucket_counts: [u64; 7],
    pub bucket_labels: [&'static str; 7],
    pub fragment_idx: usize,
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            histogram: Histogram::new_with_max(10_000_000_000, 3).unwrap(),
            ticker: Vec::with_capacity(TICKER_SIZE),
            p50_history: vec![0; SPARKLINE_HISTORY],
            p99_history: vec![0; SPARKLINE_HISTORY],
            last_tick: Instant::now(),
            start_time: Instant::now(),
            current_rate: 0,
            last_total: 0,
            peak_rate: 0,
            bucket_counts: [0; 7],
            bucket_labels: [
                "<500ns",
                "<1\u{03bc}s",
                "<2\u{03bc}s",
                "<5\u{03bc}s",
                "<10\u{03bc}s",
                "<100\u{03bc}s",
                ">100\u{03bc}s",
            ],
            fragment_idx: 1, // start at 10
        }
    }

    pub fn record(&mut self, rtt_ns: u64) {
        // Filter out invalid measurements (0 = missed pong)
        if rtt_ns == 0 {
            return;
        }
        let _ = self.histogram.record(rtt_ns.min(10_000_000_000));
        let bucket = match rtt_ns {
            0..=499 => 0,
            500..=999 => 1,
            1_000..=1_999 => 2,
            2_000..=4_999 => 3,
            5_000..=9_999 => 4,
            10_000..=99_999 => 5,
            _ => 6,
        };
        self.bucket_counts[bucket] += 1;
    }
}
