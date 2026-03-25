package com.crypto.pingpong;

import io.aeron.Aeron;
import io.aeron.driver.MediaDriver;
import io.aeron.driver.ThreadingMode;
import org.agrona.concurrent.BusySpinIdleStrategy;

import java.io.File;

/**
 * Aeron configuration for ping-pong latency benchmark.
 *
 * Two streams:
 * - Stream 1001 (PING): Java → Rust (market data)
 * - Stream 1002 (PONG): Rust → Java (echo)
 */
public class AeronConfig {

    /** IPC channel URI — shared memory transport, no network stack. */
    public static final String IPC_CHANNEL = "aeron:ipc";

    /** Ping stream: Java publishes market data → Rust receives. */
    public static final int PING_STREAM_ID = 1001;

    /** Pong stream: Rust echoes back → Java receives and measures RTT. */
    public static final int PONG_STREAM_ID = 1002;

    /** Maximum fragments to poll per subscription.poll() call. */
    public static final int FRAGMENT_LIMIT = 10;

    /** Shared memory directory — must match across all participants. */
    public static final String AERON_DIR = "/tmp/aeron-bridge";

    /** Term buffer size — 64 MB gives ample headroom for burst absorption. */
    private static final int TERM_BUFFER_LENGTH = 64 * 1024 * 1024;

    /** File page size — 4 KB matches OS page size for efficient mmap. */
    private static final int FILE_PAGE_SIZE = 4 * 1024;

    /**
     * Start embedded Media Driver optimized for latency measurement.
     *
     * <p>Uses SHARED threading mode with BusySpinIdleStrategy — a single thread
     * handles conductor, sender, and receiver duties. For a local IPC benchmark
     * where only one publication and one subscription exist, SHARED avoids the
     * overhead of three separate threads competing for cache lines, while
     * BusySpinIdleStrategy ensures the shared thread never parks.</p>
     *
     * <p>For production deployments with multiple streams, consider DEDICATED
     * threading with pinned cores for lowest jitter.</p>
     */
    public static MediaDriver startEmbeddedMediaDriver() {
        final MediaDriver.Context driverContext = new MediaDriver.Context()
            .aeronDirectoryName(AERON_DIR)
            .ipcTermBufferLength(TERM_BUFFER_LENGTH)
            .publicationTermBufferLength(TERM_BUFFER_LENGTH)
            .filePageSize(FILE_PAGE_SIZE)
            .threadingMode(ThreadingMode.SHARED)
            .sharedIdleStrategy(new BusySpinIdleStrategy())
            .errorHandler(throwable -> {
                System.err.println("Media Driver Error: " + throwable.getClass().getSimpleName()
                    + ": " + throwable.getMessage());
                throwable.printStackTrace(System.err);
            })
            // Preserve driver state across restarts so subscribers can reconnect
            // without losing position. For benchmarking, a clean start is preferred
            // — delete /tmp/aeron-bridge manually if needed.
            .dirDeleteOnStart(false)
            .dirDeleteOnShutdown(false);

        System.out.println("Starting Aeron Media Driver...");
        System.out.println("  Directory: " + AERON_DIR);
        System.out.println("  IPC Term Buffer: " + (TERM_BUFFER_LENGTH / 1024 / 1024) + " MB");
        System.out.println("  Threading Mode: " + driverContext.threadingMode());

        return MediaDriver.launch(driverContext);
    }

    /**
     * Connect to an already-running Media Driver.
     *
     * @return connected Aeron client
     * @throws IllegalStateException if Media Driver is not running
     */
    public static Aeron connectToMediaDriver() {
        final File aeronDir = new File(AERON_DIR);
        final File cncFile = new File(aeronDir, "cnc.dat");

        if (!aeronDir.exists() || !cncFile.exists()) {
            System.err.println("\n*** ERROR: Media Driver is not running! ***");
            System.err.println("Start it first: cd java && ./gradlew runMediaDriver\n");
            throw new IllegalStateException("Media Driver not running");
        }

        final Aeron.Context clientContext = new Aeron.Context()
            .aeronDirectoryName(AERON_DIR);

        System.out.println("Connecting Aeron client to Media Driver at: " + AERON_DIR);
        return Aeron.connect(clientContext);
    }
}
