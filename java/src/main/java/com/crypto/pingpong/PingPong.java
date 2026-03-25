package com.crypto.pingpong;

import io.aeron.Aeron;
import io.aeron.Publication;
import io.aeron.Subscription;
import io.aeron.logbuffer.FragmentHandler;
import org.agrona.concurrent.UnsafeBuffer;
import org.HdrHistogram.Histogram;

import java.nio.ByteBuffer;
import java.util.Random;
import java.util.concurrent.TimeUnit;

import static com.crypto.pingpong.AeronConfig.*;
import static com.crypto.pingpong.MarketDataMessage.*;

/**
 * Ping-pong latency benchmark using real market data messages.
 *
 * Flow:
 *   1. Java embeds System.nanoTime() in the timestamp field
 *   2. Publishes 48-byte market data on stream 1001 (ping)
 *   3. Rust pong echoes the raw bytes back on stream 1002
 *   4. Java receives echo, computes RTT = now - embedded timestamp
 *   5. Records RTT in HDR Histogram
 *
 * Warmup phase discards initial measurements to let JIT and caches stabilize.
 */
public class PingPong {

    private static final String[] SYMBOLS = {
        "MOON", "LAMBO", "REKT", "HODL", "PUMP", "DUMP", "DOGE", "SHIB",
        "PEPE", "WOJAK", "CHAD", "COPE", "HOPE", "NGMI", "WAGMI", "FOMO",
        "YOLO", "DEGEN", "APES", "SAFE", "DIAMOND", "PAPER", "ROCKET", "BEAR",
        "BULL", "WHALE", "SHRIMP", "BAG", "SHILL", "FUD", "ATH", "BTD",
        "SEND", "RIP", "MOON2", "COPE2", "EXIT", "LONG", "SHORT", "FLIP",
        "HYPE", "PAIN", "GAIN", "LOSS", "WIN", "FAIL", "BOOM", "BUST",
        "SQUAD", "CREW", "GANG", "FAM", "FRENS", "ANON", "BASED", "CRINGE",
        "MEME", "YEET", "BRRRR", "GUH", "STONK", "TENDIE", "BAGZ", "RAMEN"
    };

    // ---- Benchmark Configuration ----

    /** Warmup messages — discarded to let JIT compile hot paths and stabilize caches. */
    private static final int WARMUP_MESSAGES = 100_000;

    /** Measurement messages — recorded in HDR Histogram for percentile analysis. */
    private static final int MEASUREMENT_MESSAGES = 1_000_000;

    /** HDR Histogram max trackable value (10 seconds). Values above this are clamped. */
    private static final long HISTOGRAM_MAX_NS = TimeUnit.SECONDS.toNanos(10);

    /** HDR Histogram significant digits — 3 gives 0.1% precision at each percentile. */
    private static final int HISTOGRAM_PRECISION = 3;

    /** Deterministic seed for reproducible message generation across runs. */
    private static final Random random = new Random(42);

    // ---- Message Pool ----

    /** Pre-allocated message pool size — avoids GC pressure in the hot path. */
    private static final int POOL_SIZE = 10_000;
    private static final UnsafeBuffer[] messagePool = new UnsafeBuffer[POOL_SIZE];

    /** Spin-loop timeout — safety valve to prevent infinite hang if pong is lost. */
    private static final int PONG_TIMEOUT_SPINS = 10_000_000;

    /** Warmup progress report interval. */
    private static final int WARMUP_REPORT_INTERVAL = 25_000;

    /** Measurement progress report interval. */
    private static final int MEASUREMENT_REPORT_INTERVAL = 100_000;

    // ---- Volatile State (written by pong handler, read by main loop) ----
    private static volatile long pongTimestamp = 0;
    private static volatile boolean pongReceived = false;

    public static void main(String[] args) throws Exception {
        System.out.println("=== Aeron Ping-Pong: Latency Benchmark ===\n");
        System.out.println("Configuration:");
        System.out.println("  Warmup:      " + String.format("%,d", WARMUP_MESSAGES) + " messages");
        System.out.println("  Measurement: " + String.format("%,d", MEASUREMENT_MESSAGES) + " messages");
        System.out.println("  Message:     " + MESSAGE_SIZE + " bytes (market data)");
        System.out.println("  Ping stream: " + PING_STREAM_ID + " (Java → Rust)");
        System.out.println("  Pong stream: " + PONG_STREAM_ID + " (Rust → Java)");
        System.out.println();

        // Pre-generate messages
        System.out.print("Pre-generating " + POOL_SIZE + " messages... ");
        pregenerateMessages();
        System.out.println("done\n");

        // Connect to Aeron
        final Aeron aeron = connectToMediaDriver();

        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            System.out.println("\nShutting down...");
            aeron.close();
        }));

        // Create publication (ping) and subscription (pong)
        final Publication pingPub = aeron.addPublication(IPC_CHANNEL, PING_STREAM_ID);
        final Subscription pongSub = aeron.addSubscription(IPC_CHANNEL, PONG_STREAM_ID);

        // Wait for Rust pong to connect
        System.out.print("Waiting for pong subscriber on stream " + PING_STREAM_ID + "... ");
        while (!pingPub.isConnected()) {
            Thread.sleep(10);
        }
        System.out.println("connected!");

        System.out.print("Waiting for pong publisher on stream " + PONG_STREAM_ID + "... ");
        while (pongSub.imageCount() == 0) {
            Thread.sleep(10);
        }
        System.out.println("connected!\n");

        // Pong handler: extract the echoed timestamp
        final FragmentHandler pongHandler = (buffer, offset, length, header) -> {
            if (length >= MESSAGE_SIZE) {
                pongTimestamp = buffer.getLong(offset); // timestamp at offset 0
                pongReceived = true;
            }
        };

        // === WARMUP PHASE ===
        System.out.println("--- Warmup: " + String.format("%,d", WARMUP_MESSAGES) + " messages ---");
        final Histogram warmupHisto = new Histogram(HISTOGRAM_MAX_NS, HISTOGRAM_PRECISION);

        for (int i = 0; i < WARMUP_MESSAGES; i++) {
            long rtt = sendAndMeasure(pingPub, pongSub, pongHandler, i);
            if (rtt > 0) {
                warmupHisto.recordValue(Math.min(rtt, HISTOGRAM_MAX_NS));
            }

            if ((i + 1) % WARMUP_REPORT_INTERVAL == 0) {
                System.out.printf("  Warmup: %,d / %,d (p50: %,d ns, p99: %,d ns)%n",
                    i + 1, WARMUP_MESSAGES,
                    warmupHisto.getValueAtPercentile(50),
                    warmupHisto.getValueAtPercentile(99));
            }
        }
        System.out.println("  Warmup complete. Discarding warmup data.\n");

        // === MEASUREMENT PHASE ===
        System.out.println("--- Measuring: " + String.format("%,d", MEASUREMENT_MESSAGES) + " messages ---");
        final Histogram histogram = new Histogram(HISTOGRAM_MAX_NS, HISTOGRAM_PRECISION);

        final long startTime = System.nanoTime();

        for (int i = 0; i < MEASUREMENT_MESSAGES; i++) {
            long rtt = sendAndMeasure(pingPub, pongSub, pongHandler, WARMUP_MESSAGES + i);
            if (rtt > 0) {
                histogram.recordValue(Math.min(rtt, HISTOGRAM_MAX_NS));
            }

            if ((i + 1) % MEASUREMENT_REPORT_INTERVAL == 0) {
                System.out.printf("  Progress: %,d / %,d (p50: %,d ns, p99: %,d ns)%n",
                    i + 1, MEASUREMENT_MESSAGES,
                    histogram.getValueAtPercentile(50),
                    histogram.getValueAtPercentile(99));
            }
        }

        final long elapsed = System.nanoTime() - startTime;
        final double elapsedSec = elapsed / 1_000_000_000.0;
        final double rate = MEASUREMENT_MESSAGES / elapsedSec;

        // === RESULTS ===
        System.out.println("\n╔══════════════════════════════════════════════════════╗");
        System.out.println("║           PING-PONG LATENCY RESULTS                 ║");
        System.out.println("╠══════════════════════════════════════════════════════╣");
        System.out.printf("║  Messages:     %,12d                          ║%n", histogram.getTotalCount());
        System.out.printf("║  Duration:     %12.2f sec                      ║%n", elapsedSec);
        System.out.printf("║  Rate:         %,12.0f msgs/sec                 ║%n", rate);
        System.out.println("╠══════════════════════════════════════════════════════╣");
        System.out.printf("║  Min:          %,12d ns                       ║%n", histogram.getMinValue());
        System.out.printf("║  p50 (median): %,12d ns                       ║%n", histogram.getValueAtPercentile(50));
        System.out.printf("║  p90:          %,12d ns                       ║%n", histogram.getValueAtPercentile(90));
        System.out.printf("║  p99:          %,12d ns                       ║%n", histogram.getValueAtPercentile(99));
        System.out.printf("║  p99.9:        %,12d ns                       ║%n", histogram.getValueAtPercentile(99.9));
        System.out.printf("║  p99.99:       %,12d ns                       ║%n", histogram.getValueAtPercentile(99.99));
        System.out.printf("║  Max:          %,12d ns                       ║%n", histogram.getMaxValue());
        System.out.printf("║  Mean:         %,12.0f ns                       ║%n", histogram.getMean());
        System.out.printf("║  StdDev:       %,12.0f ns                       ║%n", histogram.getStdDeviation());
        System.out.println("╚══════════════════════════════════════════════════════╝");

        // Print histogram distribution
        System.out.println("\n--- Latency Distribution ---");
        System.out.println("  Value(ns)     Percentile  TotalCount");
        histogram.percentiles(5).forEach(v -> {
            if (v.getPercentile() <= 100.0) {
                System.out.printf("  %,12d  %10.4f%%  %,d%n",
                    v.getValueIteratedTo(),
                    v.getPercentile(),
                    v.getTotalCountToThisValue());
            }
        });

        Thread.sleep(500);
    }

    /**
     * Send one ping and busy-spin until the pong echo arrives.
     *
     * <p>Busy-spin is deliberate — any idle strategy (yield, park, sleep) would
     * add microseconds of wake-up latency that would dominate the sub-microsecond
     * RTT we're measuring. This matches the approach used in Aeron's own
     * ping-pong samples and LMAX Disruptor benchmarks.</p>
     *
     * @param pingPub    publication to send market data on (stream 1001)
     * @param pongSub    subscription to receive echoes on (stream 1002)
     * @param pongHandler fragment handler that sets {@code pongReceived}
     * @param sequence   message sequence number for this ping
     * @return RTT in nanoseconds, or -1 if pong was not received
     */
    private static long sendAndMeasure(
            Publication pingPub,
            Subscription pongSub,
            FragmentHandler pongHandler,
            int sequence) {

        // Get pre-allocated buffer
        final UnsafeBuffer buffer = messagePool[sequence % POOL_SIZE];

        // Embed send timestamp (nanoTime for RTT measurement)
        final long sendTime = System.nanoTime();
        buffer.putLong(0, sendTime);
        buffer.putInt(RESERVED_OFFSET, sequence);

        // Publish ping
        pongReceived = false;
        long result;
        while ((result = pingPub.offer(buffer, 0, MESSAGE_SIZE)) < 0) {
            if (result == Publication.BACK_PRESSURED || result == Publication.ADMIN_ACTION) {
                // Spin-wait on backpressure
                Thread.onSpinWait();
            } else {
                return -1; // Not connected or closed
            }
        }

        // Wait for pong (busy-spin for lowest latency)
        int spins = 0;
        while (!pongReceived) {
            pongSub.poll(pongHandler, FRAGMENT_LIMIT);
            if (++spins > PONG_TIMEOUT_SPINS) {
                // Timeout — pong not received, likely disconnected
                return -1;
            }
        }

        final long receiveTime = System.nanoTime();
        return receiveTime - sendTime;
    }

    private static void pregenerateMessages() {
        for (int i = 0; i < POOL_SIZE; i++) {
            ByteBuffer byteBuffer = ByteBuffer.allocateDirect(MESSAGE_SIZE);
            UnsafeBuffer buffer = new UnsafeBuffer(byteBuffer);

            MarketDataMessage msg = new MarketDataMessage(buffer, 0);
            String symbol = SYMBOLS[i % SYMBOLS.length];
            double price = 1.0 + random.nextDouble() * 1000.0;
            double quantity = 0.1 + random.nextDouble() * 99.9;
            double volume = price * quantity;

            msg.setTimestamp(0)
                .setSymbol(symbol)
                .setPriceFromDouble(price, EXPONENT_2)
                .setQuantityFromDouble(quantity, EXPONENT_3)
                .setVolumeFromDouble(volume, EXPONENT_2)
                .setIsBuyerMaker(random.nextBoolean());

            buffer.putInt(RESERVED_OFFSET, 0);
            messagePool[i] = buffer;
        }
    }
}
