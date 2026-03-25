//! Shared configuration and utilities for Aeron Ping-Pong latency benchmark.
//!
//! # Cross-Language Latency Measurement
//!
//! This library provides shared configuration for ping-pong latency measurement
//! between Java and Rust via Aeron IPC:
//!
//! - **Java Ping** publishes market data messages on stream 1001
//! - **Rust Pong** echoes them back immediately on stream 1002
//! - **Java** measures round-trip time using HDR Histogram
//!
//! Message format is identical to Series 1 (aeron-ipc-bridge) for consistency.

// =============================================================================
// AERON CONFIGURATION
// =============================================================================

/// Aeron Media Driver directory (must match Java Media Driver).
pub const AERON_DIR: &str = "/tmp/aeron-bridge";

/// IPC channel for shared memory transport.
pub const IPC_CHANNEL: &str = "aeron:ipc";

// =============================================================================
// PING-PONG STREAM CONFIGURATION
// =============================================================================

/// Stream ID for ping (Java → Rust): market data messages.
pub const PING_STREAM_ID: i32 = 1001;

/// Stream ID for pong (Rust → Java): echo responses.
pub const PONG_STREAM_ID: i32 = 1002;

/// Message size for cross-language communication.
/// Must match: java/src/main/java/com/crypto/pingpong/MarketDataMessage.java
///
/// Layout (matches Java MarketDataMessage exactly):
///   [0-7]   timestamp (i64 nanoseconds)
///   [8-15]  symbol (8 bytes ASCII, space-padded)
///   [16-23] price mantissa (i64)
///   [24-31] quantity mantissa (i64)
///   [32-39] volume mantissa (i64)
///   [40]    price exponent (i8)
///   [41]    quantity exponent (i8)
///   [42]    volume exponent (i8)
///   [43]    flags (u8, bit 0 = isBuyerMaker)
///   [44-47] reserved/sequence (4 bytes)
///
/// Note: Uses little-endian byte order (native on x86/ARM architectures).
pub const MESSAGE_SIZE: usize = 48;

/// Maximum fragments to poll per iteration.
pub const FRAGMENT_LIMIT: i32 = 10;

// =============================================================================
// MESSAGE FIELD OFFSETS (matching Java MarketDataMessage)
// =============================================================================

const SYMBOL_OFFSET: usize = 8;
const SYMBOL_LEN: usize = 8;
const PRICE_MANTISSA_OFFSET: usize = 16;
const QTY_MANTISSA_OFFSET: usize = 24;
#[allow(dead_code)]
const VOLUME_MANTISSA_OFFSET: usize = 32;
const PRICE_EXPONENT_OFFSET: usize = 40;
const QTY_EXPONENT_OFFSET: usize = 41;
#[allow(dead_code)]
const VOLUME_EXPONENT_OFFSET: usize = 42;
const FLAGS_OFFSET: usize = 43;

// =============================================================================
// MESSAGE ENCODING/DECODING (zero-copy, HFT-optimized)
// =============================================================================

/// Encode a timestamp (i64 nanoseconds) into the first 8 bytes of a buffer.
#[inline]
pub fn encode_timestamp(buffer: &mut [u8], timestamp_ns: i64) {
    buffer[0..8].copy_from_slice(&timestamp_ns.to_le_bytes());
}

/// Decode a timestamp (i64 nanoseconds) from the first 8 bytes of a buffer.
#[inline]
pub fn decode_timestamp(buffer: &[u8]) -> i64 {
    i64::from_le_bytes(buffer[0..8].try_into().expect("buffer too small"))
}

/// Decode symbol (8 bytes ASCII) from buffer, trimming trailing spaces.
#[inline]
pub fn decode_symbol(buffer: &[u8]) -> &str {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    let bytes = &buffer[SYMBOL_OFFSET..SYMBOL_OFFSET + SYMBOL_LEN];
    core::str::from_utf8(bytes).unwrap_or("").trim_end()
}

/// Decode price mantissa (i64) from buffer.
#[inline]
pub fn decode_price_mantissa(buffer: &[u8]) -> i64 {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    i64::from_le_bytes(
        buffer[PRICE_MANTISSA_OFFSET..PRICE_MANTISSA_OFFSET + 8]
            .try_into()
            .unwrap(),
    )
}

/// Decode quantity mantissa (i64) from buffer.
#[inline]
pub fn decode_qty_mantissa(buffer: &[u8]) -> i64 {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    i64::from_le_bytes(
        buffer[QTY_MANTISSA_OFFSET..QTY_MANTISSA_OFFSET + 8]
            .try_into()
            .unwrap(),
    )
}

/// Decode price exponent (i8) from buffer.
#[inline]
pub fn decode_price_exponent(buffer: &[u8]) -> i8 {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    buffer[PRICE_EXPONENT_OFFSET] as i8
}

/// Decode quantity exponent (i8) from buffer.
#[inline]
pub fn decode_qty_exponent(buffer: &[u8]) -> i8 {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    buffer[QTY_EXPONENT_OFFSET] as i8
}

/// Decode flags byte from buffer.
#[inline]
pub fn decode_flags(buffer: &[u8]) -> u8 {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    buffer[FLAGS_OFFSET]
}

/// Check if the trade was buyer-maker (bit 0 of flags).
#[inline]
pub fn is_buyer_maker(buffer: &[u8]) -> bool {
    debug_assert!(buffer.len() >= MESSAGE_SIZE);
    (buffer[FLAGS_OFFSET] & 0x01) != 0
}

/// Convert mantissa and exponent to f64: mantissa * 10^exponent
#[inline]
pub fn mantissa_to_f64(mantissa: i64, exponent: i8) -> f64 {
    mantissa as f64 * 10_f64.powi(exponent as i32)
}

// =============================================================================
// FORMATTING HELPERS
// =============================================================================

/// Format a number with K/M suffix for human-readable output.
pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

// =============================================================================
// UNIT TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_round_trip() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        let original_ts: i64 = 1_234_567_890_123_456_789;
        encode_timestamp(&mut buffer, original_ts);
        assert_eq!(decode_timestamp(&buffer), original_ts);
    }

    #[test]
    fn test_decode_symbol() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        buffer[8..16].copy_from_slice(b"MOON    ");
        assert_eq!(decode_symbol(&buffer), "MOON");
    }

    #[test]
    fn test_decode_price_mantissa() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        let mantissa: i64 = 3500050;
        buffer[16..24].copy_from_slice(&mantissa.to_le_bytes());
        assert_eq!(decode_price_mantissa(&buffer), mantissa);
    }

    #[test]
    fn test_mantissa_to_f64() {
        assert_eq!(mantissa_to_f64(3500050, -2), 35000.50);
        assert_eq!(mantissa_to_f64(500, -3), 0.5);
        assert_eq!(mantissa_to_f64(100, 0), 100.0);
    }

    #[test]
    fn test_is_buyer_maker() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        buffer[43] = 0x01;
        assert!(is_buyer_maker(&buffer));
        buffer[43] = 0x00;
        assert!(!is_buyer_maker(&buffer));
    }

    #[test]
    fn test_stream_ids_distinct() {
        assert_ne!(PING_STREAM_ID, PONG_STREAM_ID);
    }

    #[test]
    fn test_decode_symbol_full_width() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        buffer[8..16].copy_from_slice(b"ABCDEFGH");
        assert_eq!(decode_symbol(&buffer), "ABCDEFGH");
    }

    #[test]
    fn test_decode_symbol_all_spaces() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        buffer[8..16].copy_from_slice(b"        ");
        assert_eq!(decode_symbol(&buffer), "");
    }

    #[test]
    fn test_mantissa_to_f64_negative_mantissa() {
        assert_eq!(mantissa_to_f64(-5000, -2), -50.0);
    }

    #[test]
    fn test_mantissa_to_f64_zero() {
        assert_eq!(mantissa_to_f64(0, -2), 0.0);
    }

    #[test]
    fn test_timestamp_extremes() {
        let mut buffer = [0u8; MESSAGE_SIZE];

        encode_timestamp(&mut buffer, 0);
        assert_eq!(decode_timestamp(&buffer), 0);

        encode_timestamp(&mut buffer, i64::MAX);
        assert_eq!(decode_timestamp(&buffer), i64::MAX);

        encode_timestamp(&mut buffer, i64::MIN);
        assert_eq!(decode_timestamp(&buffer), i64::MIN);
    }

    #[test]
    fn test_flags_buyer_maker_preserves_other_bits() {
        let mut buffer = [0u8; MESSAGE_SIZE];
        buffer[43] = 0xFF; // all bits set
        assert!(is_buyer_maker(&buffer));
        buffer[43] = 0xFE; // all bits except bit 0
        assert!(!is_buyer_maker(&buffer));
    }

    #[test]
    fn test_message_size_fits_cache_line() {
        // 48 bytes fits in a single 64-byte cache line — critical for IPC performance.
        assert_eq!(MESSAGE_SIZE, 48);
    }
}
