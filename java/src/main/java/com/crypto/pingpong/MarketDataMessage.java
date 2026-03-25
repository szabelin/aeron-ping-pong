package com.crypto.pingpong;

import org.agrona.DirectBuffer;
import org.agrona.MutableDirectBuffer;
import org.agrona.concurrent.UnsafeBuffer;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;

/**
 * Custom binary message format for crypto market data (48 bytes).
 * Identical to Series 1 (aeron-ipc-bridge) for consistency.
 *
 * Message Layout (48 bytes fixed):
 * [0-7]   timestamp       - nanosecond timestamp (long)
 * [8-15]  symbol          - trading pair (8 bytes ASCII)
 * [16-23] priceMantissa   - price mantissa (long)
 * [24-31] qtyMantissa     - quantity mantissa (long)
 * [32-39] volumeMantissa  - volume mantissa (long)
 * [40]    priceExponent   - price scale (byte)
 * [41]    qtyExponent     - quantity scale (byte)
 * [42]    volumeExponent  - volume scale (byte)
 * [43]    flags           - bit 0: isBuyerMaker
 * [44-47] sequence        - message sequence number (u32)
 */
public class MarketDataMessage {

    public static final int MESSAGE_SIZE = 48;

    public static final byte EXPONENT_2 = -2;
    public static final byte EXPONENT_3 = -3;

    private static final int TIMESTAMP_OFFSET = 0;
    private static final int SYMBOL_OFFSET = 8;
    private static final int SYMBOL_LENGTH = 8;
    private static final int PRICE_MANTISSA_OFFSET = 16;
    private static final int QTY_MANTISSA_OFFSET = 24;
    private static final int VOLUME_MANTISSA_OFFSET = 32;
    private static final int PRICE_EXPONENT_OFFSET = 40;
    private static final int QTY_EXPONENT_OFFSET = 41;
    private static final int VOLUME_EXPONENT_OFFSET = 42;
    private static final int FLAGS_OFFSET = 43;
    /** Sequence number offset — used for message ordering and dedup. */
    public static final int RESERVED_OFFSET = 44;

    private static final byte IS_BUYER_MAKER_MASK = 0x01;

    private final MutableDirectBuffer buffer;
    private final int offset;

    public MarketDataMessage(MutableDirectBuffer buffer, int offset) {
        this.buffer = buffer;
        this.offset = offset;
    }

    public MarketDataMessage() {
        this(new UnsafeBuffer(ByteBuffer.allocateDirect(MESSAGE_SIZE)), 0);
    }

    // ========== ENCODER (instance methods, fluent API) ==========

    public MarketDataMessage setTimestamp(long timestamp) {
        buffer.putLong(offset + TIMESTAMP_OFFSET, timestamp);
        return this;
    }

    public MarketDataMessage setSymbol(String symbol) {
        byte[] symbolBytes = new byte[SYMBOL_LENGTH];
        byte[] input = symbol.getBytes(StandardCharsets.US_ASCII);
        int copyLen = Math.min(input.length, SYMBOL_LENGTH);
        System.arraycopy(input, 0, symbolBytes, 0, copyLen);
        for (int i = copyLen; i < SYMBOL_LENGTH; i++) {
            symbolBytes[i] = (byte) ' ';
        }
        buffer.putBytes(offset + SYMBOL_OFFSET, symbolBytes);
        return this;
    }

    public MarketDataMessage setPrice(long mantissa, byte exponent) {
        buffer.putLong(offset + PRICE_MANTISSA_OFFSET, mantissa);
        buffer.putByte(offset + PRICE_EXPONENT_OFFSET, exponent);
        return this;
    }

    public MarketDataMessage setPriceFromDouble(double price, byte exponent) {
        long mantissa = (long) (price * Math.pow(10, -exponent));
        return setPrice(mantissa, exponent);
    }

    public MarketDataMessage setQuantity(long mantissa, byte exponent) {
        buffer.putLong(offset + QTY_MANTISSA_OFFSET, mantissa);
        buffer.putByte(offset + QTY_EXPONENT_OFFSET, exponent);
        return this;
    }

    public MarketDataMessage setQuantityFromDouble(double quantity, byte exponent) {
        long mantissa = (long) (quantity * Math.pow(10, -exponent));
        return setQuantity(mantissa, exponent);
    }

    public MarketDataMessage setVolume(long mantissa, byte exponent) {
        buffer.putLong(offset + VOLUME_MANTISSA_OFFSET, mantissa);
        buffer.putByte(offset + VOLUME_EXPONENT_OFFSET, exponent);
        return this;
    }

    public MarketDataMessage setVolumeFromDouble(double volume, byte exponent) {
        long mantissa = (long) (volume * Math.pow(10, -exponent));
        return setVolume(mantissa, exponent);
    }

    public MarketDataMessage setIsBuyerMaker(boolean isBuyerMaker) {
        byte flags = buffer.getByte(offset + FLAGS_OFFSET);
        if (isBuyerMaker) {
            flags |= IS_BUYER_MAKER_MASK;
        } else {
            flags &= ~IS_BUYER_MAKER_MASK;
        }
        buffer.putByte(offset + FLAGS_OFFSET, flags);
        return this;
    }

    // ========== DECODER (static methods for zero-allocation reads) ==========
    // Static decoders read directly from any DirectBuffer without creating
    // a MarketDataMessage instance — critical for the hot path where the pong
    // handler must extract the timestamp with zero allocations.

    public static long getTimestamp(DirectBuffer buffer, int offset) {
        return buffer.getLong(offset + TIMESTAMP_OFFSET);
    }

    public static String getSymbol(DirectBuffer buffer, int offset) {
        byte[] symbolBytes = new byte[SYMBOL_LENGTH];
        buffer.getBytes(offset + SYMBOL_OFFSET, symbolBytes);
        return new String(symbolBytes, StandardCharsets.US_ASCII).trim();
    }

    public static double getPrice(DirectBuffer buffer, int offset) {
        long mantissa = buffer.getLong(offset + PRICE_MANTISSA_OFFSET);
        byte exponent = buffer.getByte(offset + PRICE_EXPONENT_OFFSET);
        return mantissa * Math.pow(10, exponent);
    }

    public static double getQuantity(DirectBuffer buffer, int offset) {
        long mantissa = buffer.getLong(offset + QTY_MANTISSA_OFFSET);
        byte exponent = buffer.getByte(offset + QTY_EXPONENT_OFFSET);
        return mantissa * Math.pow(10, exponent);
    }

    public static boolean isBuyerMaker(DirectBuffer buffer, int offset) {
        return (buffer.getByte(offset + FLAGS_OFFSET) & IS_BUYER_MAKER_MASK) != 0;
    }
}
