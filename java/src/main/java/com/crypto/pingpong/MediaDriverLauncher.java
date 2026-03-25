package com.crypto.pingpong;

import io.aeron.driver.MediaDriver;
import static com.crypto.pingpong.AeronConfig.*;

/**
 * Standalone Media Driver launcher for ping-pong latency benchmark.
 */
public class MediaDriverLauncher {

    public static void main(String[] args) {
        System.out.println("=== Aeron Ping-Pong: Media Driver ===\n");
        System.out.println("Press Ctrl+C to stop\n");

        final MediaDriver driver = startEmbeddedMediaDriver();

        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            System.out.println("\nShutting down Media Driver...");
            driver.close();
            System.out.println("Media Driver stopped");
        }));

        System.out.println("\nMedia Driver running. Ready for connections.");
        System.out.println("  Ping stream: " + PING_STREAM_ID + " (Java → Rust)");
        System.out.println("  Pong stream: " + PONG_STREAM_ID + " (Rust → Java)\n");

        try {
            Thread.currentThread().join();
        } catch (InterruptedException e) {
            // Exit
        }
    }
}
