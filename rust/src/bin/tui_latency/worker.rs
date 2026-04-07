//! Worker thread: ping sender, pong receiver, and RTT measurement.
//!
//! Runs in a dedicated thread. Sends one ping at a time on stream 1001,
//! busy-spins waiting for the pong reply on stream 1002, measures the
//! round-trip time, and forwards sampled latency data to the TUI via a
//! crossbeam channel.

use crate::state::{LatencySample, SharedParams};
use aeron_ping_pong::{
    decode_price_exponent, decode_price_mantissa, decode_qty_exponent, decode_qty_mantissa,
    decode_symbol, is_buyer_maker, mantissa_to_f64, AERON_DIR, IPC_CHANNEL, MESSAGE_SIZE,
    PING_STREAM_ID, PONG_STREAM_ID,
};
use crossbeam_channel::Sender;
use rusteron_client::*;
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// PRE-GENERATE MESSAGES (same pool approach as Java)
// =============================================================================

const POOL_SIZE: usize = 10_000;
const SYMBOLS: [&str; 64] = [
    "MOON", "LAMBO", "REKT", "HODL", "PUMP", "DUMP", "DOGE", "SHIB", "PEPE", "WOJAK", "CHAD",
    "COPE", "HOPE", "NGMI", "WAGMI", "FOMO", "YOLO", "DEGEN", "APES", "SAFE", "DIAMOND", "PAPER",
    "ROCKET", "BEAR", "BULL", "WHALE", "SHRIMP", "BAG", "SHILL", "FUD", "ATH", "BTD", "SEND",
    "RIP", "MOON2", "COPE2", "EXIT", "LONG", "SHORT", "FLIP", "HYPE", "PAIN", "GAIN", "LOSS",
    "WIN", "FAIL", "BOOM", "BUST", "SQUAD", "CREW", "GANG", "FAM", "FRENS", "ANON", "BASED",
    "CRINGE", "MEME", "YEET", "BRRRR", "GUH", "STONK", "TENDIE", "BAGZ", "RAMEN",
];

fn pregenerate_messages() -> Vec<[u8; MESSAGE_SIZE]> {
    let mut pool = Vec::with_capacity(POOL_SIZE);
    let mut rng_state: u64 = 42;

    for i in 0..POOL_SIZE {
        let mut buf = [0u8; MESSAGE_SIZE];

        // Symbol
        let sym = SYMBOLS[i % SYMBOLS.len()].as_bytes();
        let mut sym_field = [b' '; 8];
        sym_field[..sym.len().min(8)].copy_from_slice(&sym[..sym.len().min(8)]);
        buf[8..16].copy_from_slice(&sym_field);

        // Simple PRNG for price/qty
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let price_mantissa = ((rng_state >> 16) % 100000) as i64 + 100;
        buf[16..24].copy_from_slice(&price_mantissa.to_le_bytes());

        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let qty_mantissa = ((rng_state >> 16) % 99900) as i64 + 100;
        buf[24..32].copy_from_slice(&qty_mantissa.to_le_bytes());

        // Exponents: -2 for price, -3 for qty
        buf[40] = 0xFE_u8; // -2 as i8
        buf[41] = 0xFD_u8; // -3 as i8

        // Flags: alternating buy/sell
        buf[43] = if i % 2 == 0 { 0x01 } else { 0x00 };

        pool.push(buf);
    }
    pool
}

// =============================================================================
// MONOTONIC CLOCK
// =============================================================================

// Monotonic clock using quanta — reads TSC directly, ~3-5ns vs ~25ns for
// std::Instant::now() which goes through clock_gettime(CLOCK_MONOTONIC).
// Still monotonic and immune to wall-clock adjustments (NTP, daylight savings).
use std::sync::OnceLock;
static CLOCK: OnceLock<quanta::Clock> = OnceLock::new();
static CLOCK_START: OnceLock<quanta::Instant> = OnceLock::new();

fn init_clock() {
    let clock = CLOCK.get_or_init(quanta::Clock::new);
    CLOCK_START.get_or_init(|| clock.now());
}

#[inline]
fn nanos_since_start() -> u64 {
    let clock = CLOCK.get().unwrap();
    let start = CLOCK_START.get().unwrap();
    clock.now().duration_since(*start).as_nanos() as u64
}

// =============================================================================
// WORKER THREAD (ping sender + pong receiver + RTT measurement)
// =============================================================================

pub fn worker_thread(params: Arc<SharedParams>, sample_tx: Sender<LatencySample>) {
    let ctx = match AeronContext::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create Aeron context: {}", e);
            params.running.store(false, Ordering::SeqCst);
            return;
        }
    };

    let aeron_dir = CString::new(AERON_DIR).unwrap();
    if let Err(e) = ctx.set_dir(&aeron_dir) {
        eprintln!("Failed to set dir: {}", e);
        params.running.store(false, Ordering::SeqCst);
        return;
    }

    let aeron = match Aeron::new(&ctx) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to create Aeron: {}", e);
            params.running.store(false, Ordering::SeqCst);
            return;
        }
    };

    if let Err(e) = aeron.start() {
        eprintln!("Failed to start: {}", e);
        params.running.store(false, Ordering::SeqCst);
        return;
    }

    let channel = CString::new(IPC_CHANNEL).unwrap();

    // Publication on ping stream (1001)
    let publication = match aeron
        .async_add_publication(&channel, PING_STREAM_ID)
        .and_then(|p| p.poll_blocking(Duration::from_secs(10)))
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create publication: {}", e);
            params.running.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Subscription on pong stream (1002)
    let subscription = match aeron
        .async_add_subscription(
            &channel,
            PONG_STREAM_ID,
            Handlers::no_available_image_handler(),
            Handlers::no_unavailable_image_handler(),
        )
        .and_then(|s| s.poll_blocking(Duration::from_secs(10)))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create subscription: {}", e);
            params.running.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Pre-generate message pool
    let pool = pregenerate_messages();

    // Pong handler state
    struct PongHandler {
        tx: Sender<LatencySample>,
        received: std::cell::Cell<u64>,
        dropped: std::cell::Cell<u64>,
        last_rtt: std::cell::Cell<u64>,
        got_pong: std::cell::Cell<bool>,
    }

    impl AeronFragmentHandlerCallback for PongHandler {
        fn handle_aeron_fragment_handler(&mut self, buffer: &[u8], _header: AeronHeader) {
            if buffer.len() < MESSAGE_SIZE {
                return;
            }

            let count = self.received.get() + 1;
            self.received.set(count);
            self.got_pong.set(true);

            // Decode the echoed timestamp -- our monotonic clock serialized as nanos
            let send_nanos = i64::from_le_bytes(buffer[0..8].try_into().unwrap()) as u64;
            let now_nanos = nanos_since_start();
            // saturating_sub handles any clock weirdness safely
            let rtt = now_nanos.saturating_sub(send_nanos);
            self.last_rtt.set(rtt);

            // Send every 100th sample to TUI for ticker display
            if count % 100 == 0 {
                let symbol_str = decode_symbol(buffer);
                let price =
                    mantissa_to_f64(decode_price_mantissa(buffer), decode_price_exponent(buffer));
                let qty = mantissa_to_f64(decode_qty_mantissa(buffer), decode_qty_exponent(buffer));
                let is_buy = is_buyer_maker(buffer);

                let mut symbol = [b' '; 8];
                let bytes = symbol_str.as_bytes();
                symbol[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);

                if self
                    .tx
                    .try_send(LatencySample {
                        rtt_ns: rtt,
                        symbol,
                        price,
                        qty,
                        is_buy,
                        seq: count,
                    })
                    .is_err()
                {
                    self.dropped.set(self.dropped.get() + 1);
                }
            }
        }
    }

    let (handler, inner) = match Handler::leak_with_fragment_assembler(PongHandler {
        tx: sample_tx,
        received: std::cell::Cell::new(0),
        dropped: std::cell::Cell::new(0),
        last_rtt: std::cell::Cell::new(0),
        got_pong: std::cell::Cell::new(false),
    }) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to create handler: {}", e);
            params.running.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Initialize the monotonic clock baseline
    init_clock();

    let mut seq: u64 = 0;

    // Main ping-pong loop: send ONE, wait for response, measure, repeat.
    // This is the only way to get clean RTT -- no batching on the critical path.
    while params.running.load(Ordering::Relaxed) {
        if params.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        let frag_limit = params.fragment_limit.load(Ordering::Relaxed);

        // --- SEND ONE PING ---
        let buf_idx = (seq as usize) % POOL_SIZE;
        let mut msg = pool[buf_idx];

        // Embed send time as monotonic nanos (u64, safe for ~584 years)
        let send_time = nanos_since_start();
        msg[0..8].copy_from_slice(&(send_time as i64).to_le_bytes());
        msg[44..48].copy_from_slice(&(seq as u32).to_le_bytes());

        let result = publication.offer(&msg, Handlers::no_reserved_value_supplier_handler());

        if result <= 0 {
            // Backpressure or not connected -- spin and retry
            std::hint::spin_loop();
            continue;
        }

        seq += 1;
        params.total_sent.fetch_add(1, Ordering::Relaxed);

        // --- WAIT FOR PONG (busy-spin with time-based timeout) ---
        // NOTE: We intentionally busy-spin here instead of using BackoffIdleStrategy
        // (as Aeron's ping.rs and our Series 1 consumer do). BackoffIdleStrategy
        // parks the thread for 1-100us after exhausting spins -- at p50=375ns RTT,
        // even a 1us park would 3x the measured latency, contaminating the histogram
        // with idle overhead rather than pure transport time.
        // For production: BackoffIdleStrategy (saves CPU).
        // For benchmarking: busy-spin (standard practice, matches Aeron/LMAX examples).
        inner.got_pong.set(false);
        let clock = CLOCK.get().unwrap();
        let wait_start = clock.now();
        let timeout = Duration::from_millis(100); // 100ms timeout -- generous for IPC

        while !inner.got_pong.get() {
            subscription.poll(Some(&handler), frag_limit as usize).ok();
            if clock.now().duration_since(wait_start) > timeout {
                params.timeouts.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }

        params
            .total_received
            .store(inner.received.get(), Ordering::Relaxed);
        params
            .dropped_samples
            .store(inner.dropped.get(), Ordering::Relaxed);
    }
}
