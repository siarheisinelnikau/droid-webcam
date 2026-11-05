package com.droidwebcam;

import android.annotation.SuppressLint;
import android.graphics.ImageFormat;
import android.graphics.Rect;
import android.graphics.YuvImage;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.media.Image;
import android.media.ImageReader;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Range;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

final class PhoneCamera {
    private final CaptureSettings settings;
    private final String cameraId;
    private final Range<Integer> fps;
    private final long frameNs;
    private final CameraManager manager;
    private final Consumer<byte[]> output;
    private final Consumer<String> failure;
    private final HandlerThread thread = new HandlerThread("Droid Webcam camera");
    private final Handler handler;
    private final HandlerThread encoderThread = new HandlerThread("Droid Webcam encoder");
    private final Handler encoder;
    private final AtomicBoolean stopping = new AtomicBoolean();
    private final byte[] nv21;
    private final ByteArrayOutputStream jpeg = new ByteArrayOutputStream(64 * 1024);
    private ImageReader reader;
    private CameraDevice camera;
    private CameraCaptureSession session;
    private boolean opening;
    private long nextFrame;
    private int oversized;

    PhoneCamera(CameraManager manager, CameraOptions.Camera selectedCamera, CameraOptions.Mode mode,
            CaptureSettings settings, Consumer<byte[]> output, Consumer<String> failure) {
        this.settings = settings;
        this.cameraId = selectedCamera.id;
        this.fps = mode.aeRange;
        this.frameNs = 1_000_000_000L / settings.fps;
        this.nv21 = new byte[settings.width * settings.height * 3 / 2];
        this.manager = manager;
        this.output = output;
        this.failure = failure;
        thread.start();
        handler = new Handler(thread.getLooper());
        encoderThread.start();
        encoder = new Handler(encoderThread.getLooper());
    }

    void start() { handler.post(this::open); }

    void stop() {
        if (stopping.compareAndSet(false, true)) handler.post(this::release);
    }

    @SuppressLint("MissingPermission") // The activity checks CAMERA before creating a session.
    private void open() {
        if (stopping.get()) return;
        try {
            reader = ImageReader.newInstance(settings.width, settings.height, ImageFormat.YUV_420_888, 3);
            reader.setOnImageAvailableListener(this::onImage, encoder);
            manager.openCamera(cameraId, deviceCallback, handler);
            opening = true;
        } catch (Exception error) {
            fail("Could not open the camera: " + error.getMessage());
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
            fail("Camera disconnected.");
        }

        @Override public void onError(CameraDevice device, int error) {
            opening = false;
            camera = device;
            fail("Camera error (" + error + ").");
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
                CaptureRequest.Builder request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
                request.addTarget(reader.getSurface());
                if (fps != null) request.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, fps);
                session.setRepeatingRequest(request.build(), null, handler);
            } catch (Exception error) {
                fail("Could not start capture: " + error.getMessage());
            }
        }

        @Override public void onConfigureFailed(CameraCaptureSession configured) {
            configured.close();
            fail("The camera rejected the capture mode.");
        }
    };

    private void onImage(ImageReader source) {
        if (stopping.get()) return;
        try (Image image = source.acquireLatestImage()) {
            if (image == null) return;
            if (image.getWidth() != settings.width || image.getHeight() != settings.height
                    || !image.getCropRect().equals(new Rect(0, 0, settings.width, settings.height))) {
                throw new IOException("Camera returned an unexpected frame size");
            }
            long now = image.getTimestamp();
            if (now == 0) now = System.nanoTime();
            // Use sensor time so encoder scheduling jitter does not halve the selected FPS.
            if (now + frameNs / 8 < nextFrame) return;
            nextFrame = Math.max(nextFrame + frameNs, now + frameNs * 3 / 4);
            copyToNv21(image);
            jpeg.reset();
            if (!new YuvImage(nv21, ImageFormat.NV21, settings.width, settings.height, null)
                    .compressToJpeg(new Rect(0, 0, settings.width, settings.height), settings.quality, jpeg)) {
                throw new IOException("JPEG encoder failed");
            }
            if (jpeg.size() <= Connection.MAX_JPEG) {
                oversized = 0;
                output.accept(jpeg.toByteArray());
            } else if (++oversized >= settings.fps) {
                fail("Frames exceed the transport limit. Choose a lower image quality.");
            }
        } catch (Exception error) {
            fail("Capture failed: " + error.getMessage());
        }
    }

    private void copyToNv21(Image image) {
        Image.Plane[] planes = image.getPlanes();
        ByteBuffer y = planes[0].getBuffer();
        int yBase = y.position();
        for (int row = 0; row < settings.height; row++) {
            y.position(yBase + row * planes[0].getRowStride());
            y.get(nv21, row * settings.width, settings.width);
        }
        ByteBuffer u = planes[1].getBuffer();
        ByteBuffer v = planes[2].getBuffer();
        int uBase = u.position();
        int vBase = v.position();
        int offset = settings.width * settings.height;
        for (int row = 0; row < settings.height / 2; row++) {
            int uRow = uBase + row * planes[1].getRowStride();
            int vRow = vBase + row * planes[2].getRowStride();
            for (int col = 0; col < settings.width / 2; col++) {
                nv21[offset++] = v.get(vRow + col * planes[2].getPixelStride());
                nv21[offset++] = u.get(uRow + col * planes[1].getPixelStride());
            }
        }
    }

    private void fail(String message) {
        if (stopping.compareAndSet(false, true)) failure.accept(message);
        // Encoding errors arrive on a separate thread. Camera state is owned by its handler.
        handler.post(this::release);
    }

    private void release() {
        if (session != null) { session.close(); session = null; }
        if (camera != null) { camera.close(); camera = null; }
        ImageReader closing = reader;
        reader = null;
        if (closing != null) {
            encoder.post(() -> { closing.setOnImageAvailableListener(null, null); closing.close(); });
        }
        encoderThread.quitSafely();
        // Wait for CameraDevice's callback so we can close a still-opening device.
        if (!opening) thread.quitSafely();
    }
}
