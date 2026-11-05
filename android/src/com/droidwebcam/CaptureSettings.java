package com.droidwebcam;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.Arrays;

final class CaptureSettings {
    final int width;
    final int height;
    final int fps;
    final int quality;
    final boolean front;

    CaptureSettings(int width, int height, int fps, int quality, boolean front) throws IOException {
        if (!((width == 640 && height == 480) || (width == 1280 && height == 720)
                || (width == 1920 && height == 1080)) || (fps != 15 && fps != 30 && fps != 60)
                || quality < 40 || quality > 95) {
            throw new IOException("Unsupported capture settings.");
        }
        this.width = width;
        this.height = height;
        this.fps = fps;
        this.quality = quality;
        this.front = front;
    }

    static CaptureSettings defaults() throws IOException { return new CaptureSettings(1280, 720, 30, 80, false); }
    static CaptureSettings legacy() throws IOException { return new CaptureSettings(640, 480, 15, 75, false); }

    static boolean supportsFrameDuration(long minimumDuration, int fps) {
        return fps > 0 && (minimumDuration == 0 || minimumDuration <= (1_000_000_000L + fps - 1) / fps);
    }

    byte[] wire() {
        return ByteBuffer.allocate(8).putShort((short) width).putShort((short) height)
                .putShort((short) fps).put((byte) quality).put((byte) (front ? 1 : 0)).array();
    }

    boolean matches(byte[] accepted) { return Arrays.equals(wire(), accepted); }
}
