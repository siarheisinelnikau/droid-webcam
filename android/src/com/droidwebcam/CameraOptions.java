package com.droidwebcam;

import android.graphics.ImageFormat;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.util.Range;
import android.util.Size;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

final class CameraOptions {
    static final class Mode {
        final Size size;
        final int fps;
        final Range<Integer> aeRange;

        Mode(Size size, int fps, Range<Integer> aeRange) {
            this.size = size;
            this.fps = fps;
            this.aeRange = aeRange;
        }
    }

    static final class Camera {
        final String id;
        final boolean front;
        final List<Mode> modes;

        Camera(String id, boolean front, List<Mode> modes) {
            this.id = id;
            this.front = front;
            this.modes = modes;
        }

        @Override public String toString() { return (front ? "Front" : "Rear") + " (" + id + ")"; }
    }

    static List<Camera> enumerate(CameraManager manager) throws Exception {
        List<Camera> cameras = new ArrayList<>();
        for (String id : manager.getCameraIdList()) {
            CameraCharacteristics info = manager.getCameraCharacteristics(id);
            Integer facing = info.get(CameraCharacteristics.LENS_FACING);
            if (facing == null || (facing != CameraCharacteristics.LENS_FACING_BACK
                    && facing != CameraCharacteristics.LENS_FACING_FRONT)) continue;
            StreamConfigurationMap map = info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
            if (map == null) continue;
            Size[] supported = map.getOutputSizes(ImageFormat.YUV_420_888);
            Range<Integer>[] ranges = info.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES);
            if (supported == null || ranges == null) continue;
            List<Mode> modes = new ArrayList<>();
            for (Size size : new Size[] { new Size(1280, 720), new Size(640, 480), new Size(1920, 1080) }) {
                if (!Arrays.asList(supported).contains(size)) continue;
                long minimumDuration = map.getOutputMinFrameDuration(ImageFormat.YUV_420_888, size);
                for (int fps : new int[] { 30, 60, 15 }) {
                    // Metadata can round the duration by a nanosecond.
                    if (!CaptureSettings.supportsFrameDuration(minimumDuration, fps)) continue;
                    Range<Integer> selected = null;
                    int best = Integer.MAX_VALUE;
                    for (Range<Integer> range : ranges) {
                        if (range.getUpper() < fps) continue;
                        if (!CaptureSettings.supportsFrameDuration(minimumDuration, range.getLower())) continue;
                        int score = (range.getUpper() - fps) * 1000 + Math.abs(range.getLower() - fps);
                        if (score < best) { selected = range; best = score; }
                    }
                    if (selected != null) modes.add(new Mode(size, fps, selected));
                }
            }
            if (!modes.isEmpty()) cameras.add(new Camera(id, facing == CameraCharacteristics.LENS_FACING_FRONT, modes));
        }
        cameras.sort((left, right) -> Boolean.compare(left.front, right.front));
        return cameras;
    }
}
