//! Rust pong echo service for latency measurement.
//!
//! Subscribes to market data on stream 1001 (ping),
//! immediately echoes the raw 48 bytes back on stream 1002 (pong).
//! Zero processing — the fastest possible echo for clean RTT measurement.
//!
//! # Design Choices
//!
//! - **Zero-copy echo**: raw bytes are forwarded without decoding. Any processing
//!   in the echo path adds noise to RTT, so we keep it minimal.
//! - **Busy-spin polling**: `std::hint::spin_loop()` keeps the thread on-core
//!   for lowest possible wake-up latency. Uses 100% of one CPU core.
//! - **Fragment assembler**: wraps the handler to reassemble messages that span
//!   multiple Aeron fragments (unlikely at 48 bytes, but correct by default).

use aeron_ping_pong::{
    format_count, AERON_DIR, FRAGMENT_LIMIT, IPC_CHANNEL, MESSAGE_SIZE, PING_STREAM_ID,
    PONG_STREAM_ID,
};
use rusteron_client::*;
use std::cell::Cell;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Aeron Ping-Pong: Rust Pong (Echo) ===\n");

    // Graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\nCtrl+C received, shutting down...");
        r.store(false, Ordering::SeqCst);
    })?;

    // Connect to Aeron Media Driver
    println!("Connecting to Media Driver at: {}", AERON_DIR);

    let ctx = AeronContext::new()?;
    let aeron_dir = CString::new(AERON_DIR)?;
    ctx.set_dir(&aeron_dir)?;

    let aeron = Aeron::new(&ctx)?;
    aeron.start()?;
    println!("Connected to Aeron");

    // Create subscription on ping stream (1001) — receive market data
    println!("Subscribing to ping stream {}", PING_STREAM_ID);
    let channel = CString::new(IPC_CHANNEL)?;
    let subscription = aeron
        .async_add_subscription(
            &channel,
            PING_STREAM_ID,
            Handlers::no_available_image_handler(),
            Handlers::no_unavailable_image_handler(),
        )?
        .poll_blocking(Duration::from_secs(10))?;

    // Create publication on pong stream (1002) — echo back
    println!("Publishing pong on stream {}", PONG_STREAM_ID);
    let publication = aeron
        .async_add_publication(&channel, PONG_STREAM_ID)?
        .poll_blocking(Duration::from_secs(10))?;

    println!(
        "Ready — echoing messages from stream {} → stream {}\n",
        PING_STREAM_ID, PONG_STREAM_ID
    );

    // Fragment handler: receive ping, immediately echo pong.
    // Uses Cell<u64> for interior mutability — safe because the handler
    // is only called from the single polling thread (no Send/Sync needed).
    struct EchoHandler {
        publication: AeronPublication,
        echo_count: Cell<u64>,
        fail_count: Cell<u64>,
    }

    impl AeronFragmentHandlerCallback for EchoHandler {
        fn handle_aeron_fragment_handler(&mut self, buffer: &[u8], _header: AeronHeader) {
            if buffer.len() < MESSAGE_SIZE {
                return;
            }

            // Echo the raw bytes back immediately — zero processing for cleanest latency
            let result = self
                .publication
                .offer(buffer, Handlers::no_reserved_value_supplier_handler());

            if result > 0 {
                self.echo_count.set(self.echo_count.get() + 1);
            } else {
                self.fail_count.set(self.fail_count.get() + 1);
            }
        }
    }

    let (handler, inner) = Handler::leak_with_fragment_assembler(EchoHandler {
        publication,
        echo_count: Cell::new(0),
        fail_count: Cell::new(0),
    })?;

    // Main poll loop — busy spin for lowest latency
    let mut last_report = std::time::Instant::now();
    let mut last_count = 0u64;

    while running.load(Ordering::Relaxed) {
        subscription.poll(Some(&handler), FRAGMENT_LIMIT as usize)?;

        // Stats every 5 seconds
        if last_report.elapsed() >= Duration::from_secs(5) {
            let count = inner.echo_count.get();
            let fails = inner.fail_count.get();
            let rate = count - last_count;
            if count > 0 {
                println!(
                    "Echoed: {:>12} total  ({:>10}/sec)  |  Failed: {}",
                    format_count(count),
                    format_count(rate / 5),
                    fails
                );
            }
            last_count = count;
            last_report = std::time::Instant::now();
        }

        // Spin — no sleep for lowest latency
        std::hint::spin_loop();
    }

    println!("\n=== Pong shutting down ===");
    println!("Total echoed: {}", inner.echo_count.get());
    println!("Total failed: {}", inner.fail_count.get());

    Ok(())
}

