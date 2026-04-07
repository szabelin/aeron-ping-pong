use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hdrhistogram::Histogram;

// -----------------------------------------------------------------------------
// Message encoding — runs on every ping send
// -----------------------------------------------------------------------------

fn bench_encode_timestamp(c: &mut Criterion) {
    let mut buf = [0u8; 48];

    c.bench_function("encode_timestamp", |b| {
        b.iter(|| {
            aeron_ping_pong::encode_timestamp(&mut buf, black_box(123_456_789));
        })
    });
}

fn bench_encode_full_message(c: &mut Criterion) {
    let mut buf = [0u8; 48];

    c.bench_function("encode_full_message", |b| {
        b.iter(|| {
            aeron_ping_pong::encode_timestamp(&mut buf, black_box(123_456_789));
            buf[8..16].copy_from_slice(b"MOON    ");
            buf[16..24].copy_from_slice(&42_00i64.to_le_bytes());
            buf[24..32].copy_from_slice(&100_000i64.to_le_bytes());
            buf[32..40].copy_from_slice(&0i64.to_le_bytes());
            buf[40] = -2i8 as u8;
            buf[41] = -3i8 as u8;
            buf[42] = 0;
            buf[43] = 1;
            buf[44..48].copy_from_slice(&1u32.to_le_bytes());
            black_box(&buf);
        })
    });
}

// -----------------------------------------------------------------------------
// Message decoding — runs on every pong receive
// -----------------------------------------------------------------------------

fn bench_decode_all_fields(c: &mut Criterion) {
    let mut buf = [0u8; 48];
    aeron_ping_pong::encode_timestamp(&mut buf, 123_456_789);
    buf[8..16].copy_from_slice(b"MOON    ");
    buf[16..24].copy_from_slice(&42_00i64.to_le_bytes());
    buf[24..32].copy_from_slice(&100_000i64.to_le_bytes());
    buf[40] = -2i8 as u8;
    buf[41] = -3i8 as u8;
    buf[43] = 1;

    c.bench_function("decode_all_fields", |b| {
        b.iter(|| {
            let ts = aeron_ping_pong::decode_timestamp(black_box(&buf));
            let sym = aeron_ping_pong::decode_symbol(&buf);
            let price = aeron_ping_pong::decode_price_mantissa(&buf);
            let qty = aeron_ping_pong::decode_qty_mantissa(&buf);
            let flags = aeron_ping_pong::decode_flags(&buf);
            let is_buy = aeron_ping_pong::is_buyer_maker(&buf);
            black_box((ts, sym, price, qty, flags, is_buy));
        })
    });
}

// -----------------------------------------------------------------------------
// HDR Histogram — runs on every RTT measurement
// -----------------------------------------------------------------------------

fn bench_histogram_record(c: &mut Criterion) {
    let mut hist = Histogram::<u64>::new_with_max(10_000_000_000, 3).unwrap();

    c.bench_function("histogram_record", |b| {
        let mut val = 1_000u64;
        b.iter(|| {
            hist.record(black_box(val)).unwrap();
            val = val.wrapping_add(7);
        })
    });
}

fn bench_histogram_record_corrected(c: &mut Criterion) {
    let mut hist = Histogram::<u64>::new_with_max(10_000_000_000, 3).unwrap();
    let expected_interval = 1_000_000u64; // 1ms

    c.bench_function("histogram_record_corrected", |b| {
        let mut val = 1_000u64;
        b.iter(|| {
            hist.record_correct(black_box(val), expected_interval).unwrap();
            val = val.wrapping_add(7);
        })
    });
}

// -----------------------------------------------------------------------------
// Clock — baseline for later quanta comparison
// -----------------------------------------------------------------------------

fn bench_std_instant_now(c: &mut Criterion) {
    c.bench_function("std::Instant::now", |b| {
        b.iter(|| {
            black_box(std::time::Instant::now());
        })
    });
}

criterion_group!(
    benches,
    bench_encode_timestamp,
    bench_encode_full_message,
    bench_decode_all_fields,
    bench_histogram_record,
    bench_histogram_record_corrected,
    bench_std_instant_now,
);
criterion_main!(benches);
