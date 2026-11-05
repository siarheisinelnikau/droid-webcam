package com.droidwebcam;

import android.annotation.SuppressLint;
import android.graphics.ImageFormat;
import android.graphics.Rect;
import android.graphics.YuvImage;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Range;
import android.util.Size;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.Collections;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

final class RearCamera {
    private static final int WIDTH = 640;
    private static final int HEIGHT = 480;
    private static final long FRAME_NS = 1_000_000_000L / 15;
    private final CameraManager manager;
    private final Consumer<byte[]> output;
    private final Consumer<String> failure;
    private final HandlerThread thread = new HandlerThread("Droid Webcam camera");
    private final Handler handler;
    private final AtomicBoolean stopping = new AtomicBoolean();
    private final byte[] nv21 = new byte[WIDTH * HEIGHT * 3 / 2];
    private final ByteArrayOutputStream jpeg = new ByteArrayOutputStream(64 * 1024);
    private ImageReader reader;
    private CameraDevice camera;
    private CameraCaptureSession session;
    private Range<Integer> fps;
    private boolean opening;
    private long nextFrame;
    private int oversized;

    RearCamera(CameraManager manager, Consumer<byte[]> output, Consumer<String> failure) {
        this.manager = manager;
        this.output = output;
        this.failure = failure;
        thread.start();
        handler = new Handler(thread.getLooper());
    }

    void start() { handler.post(this::open); }

    void stop() {
        if (stopping.compareAndSet(false, true)) handler.post(this::release);
    }

    @SuppressLint("MissingPermission") // The activity checks CAMERA before creating a session.
    private void open() {
        if (stopping.get()) return;
        try {
            for (String id : manager.getCameraIdList()) {
                CameraCharacteristics info = manager.getCameraCharacteristics(id);
                if (!Integer.valueOf(CameraCharacteristics.LENS_FACING_BACK)
                        .equals(info.get(CameraCharacteristics.LENS_FACING))) continue;
                StreamConfigurationMap map = info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
                Size[] sizes = map == null ? null : map.getOutputSizes(ImageFormat.YUV_420_888);
                if (sizes == null || !Arrays.asList(sizes).contains(new Size(WIDTH, HEIGHT))) continue;
                Range<Integer>[] ranges = info.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES);
                Range<Integer> fixed = new Range<>(15, 15);
                if (ranges != null && Arrays.asList(ranges).contains(fixed)) fps = fixed;
                reader = ImageReader.newInstance(WIDTH, HEIGHT, ImageFormat.YUV_420_888, 2);
                reader.setOnImageAvailableListener(this::onImage, handler);
                manager.openCamera(id, deviceCallback, handler);
                opening = true;
                return;
            }
            fail("No rear camera supports 640×480 YUV.");
        } catch (Exception error) {
            fail("Could not open the rear camera: " + error.getMessage());
        }
    }

    private final CameraDevice.StateCallback deviceCallback = new CameraDevice.StateCallback() {
        @Override public void onOpened(CameraDevice device) {
            opening = false;
            camera = device;
            if (stopping.get()) {
                release();
                return;
            }
            try {
                device.createCaptureSession(Collections.singletonList(reader.getSurface()), captureCallback, handler);
            } catch (Exception error) {
                fail("Could not configure the camera: " + error.getMessage());
            }
        }

        @Override public void onDisconnected(CameraDevice device) {
            opening = false;
            camera = device;
            fail("Rear camera disconnected.");
        }

        @Override public void onError(CameraDevice device, int error) {
            opening = false;
            camera = device;
            fail("Rear camera error (" + error + ").");
        }
    };

    private final CameraCaptureSession.StateCallback captureCallback = new CameraCaptureSession.StateCallback() {
        @Override public void onConfigured(CameraCaptureSession configured) {
            if (stopping.get()) {
                configured.close();
                return;
            }
            session = configured;
            try {
                CaptureRequest.Builder request = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
                request.addTarget(reader.getSurface());
                if (fps != null) request.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, fps);
                session.setRepeatingRequest(request.build(), null, handler);
            } catch (Exception error) {
                fail("Could not start capture: " + error.getMessage());
            }
        }

        @Override public void onConfigureFailed(CameraCaptureSession configured) {
            configured.close();
            fail("The rear camera rejected the capture mode.");
        }
    };

    private void onImage(ImageReader source) {
        if (stopping.get()) return;
        try (Image image = source.acquireLatestImage()) {
            if (image == null) return;
            long now = System.nanoTime();
            if (now < nextFrame) return;
            // Preserve cadence across small callback jitter without accumulating old frames.
            nextFrame = Math.max(now - FRAME_NS / 4, nextFrame) + FRAME_NS;
            copyToNv21(image);
            jpeg.reset();
            if (!new YuvImage(nv21, ImageFormat.NV21, WIDTH, HEIGHT, null)
                    .compressToJpeg(new Rect(0, 0, WIDTH, HEIGHT), 75, jpeg)) {
                throw new IOException("JPEG encoder failed");
            }
            if (jpeg.size() <= Connection.MAX_JPEG) {
                oversized = 0;
                output.accept(jpeg.toByteArray());
            } else if (++oversized >= 15) {
                fail("The camera repeatedly produced oversized frames.");
            }
        } catch (Exception error) {
            fail("Capture failed: " + error.getMessage());
        }
    }

    private void copyToNv21(Image image) {
        Image.Plane[] planes = image.getPlanes();
        ByteBuffer y = planes[0].getBuffer();
        int yBase = y.position();
        for (int row = 0; row < HEIGHT; row++) {
            y.position(yBase + row * planes[0].getRowStride());
            y.get(nv21, row * WIDTH, WIDTH);
        }
        ByteBuffer u = planes[1].getBuffer();
        ByteBuffer v = planes[2].getBuffer();
        int uBase = u.position();
        int vBase = v.position();
        int offset = WIDTH * HEIGHT;
        for (int row = 0; row < HEIGHT / 2; row++) {
            int uRow = uBase + row * planes[1].getRowStride();
            int vRow = vBase + row * planes[2].getRowStride();
            for (int col = 0; col < WIDTH / 2; col++) {
                nv21[offset++] = v.get(vRow + col * planes[2].getPixelStride());
                nv21[offset++] = u.get(uRow + col * planes[1].getPixelStride());
            }
        }
    }

    private void fail(String message) {
        if (stopping.compareAndSet(false, true)) failure.accept(message);
        release();
    }

    private void release() {
        if (session != null) { session.close(); session = null; }
        if (camera != null) { camera.close(); camera = null; }
        if (reader != null) { reader.close(); reader = null; }
        // Wait for CameraDevice's callback so we can close a still-opening device.
        if (!opening) thread.quitSafely();
    }
}
