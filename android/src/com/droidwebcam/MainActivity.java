package com.droidwebcam;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.hardware.camera2.CameraManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputFilter;
import android.text.InputType;
import android.util.Size;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Spinner;
import android.widget.TextView;
import java.io.IOException;
import java.net.Socket;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import javax.net.ssl.SSLHandshakeException;

public final class MainActivity extends Activity {
    private static final int CAMERA_PERMISSION = 1;
    private static final int[] QUALITIES = { 55, 80, 95 };
    private final Handler ui = new Handler(Looper.getMainLooper());
    private EditText invitation;
    private TextView status;
    private Button start;
    private Button stop;
    private Spinner cameraChoice;
    private Spinner resolutionChoice;
    private Spinner fpsChoice;
    private Spinner qualityChoice;
    private Session current;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        int padding = Math.round(16 * getResources().getDisplayMetrics().density);
        content.setPadding(padding, padding, padding, padding);
        content.setOnApplyWindowInsetsListener((view, insets) -> {
            view.setPadding(padding + insets.getSystemWindowInsetLeft(),
                    padding + insets.getSystemWindowInsetTop(), padding + insets.getSystemWindowInsetRight(),
                    padding + insets.getSystemWindowInsetBottom());
            return insets;
        });
        TextView title = new TextView(this);
        title.setText("Droid Webcam");
        title.setTextSize(24);
        content.addView(title);
        TextView instructions = new TextView(this);
        instructions.setText("Join the PC’s Wi-Fi. Choose a camera and video settings, paste its invitation, then tap Start.");
        content.addView(instructions);
        invitation = new EditText(this);
        invitation.setHint("One-time invitation from Windows or Ubuntu");
        invitation.setSingleLine();
        invitation.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        invitation.setImeOptions(EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        invitation.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
        invitation.setSaveEnabled(false);
        invitation.setFilters(new InputFilter[] { new InputFilter.LengthFilter(192) });
        content.addView(invitation);
        LinearLayout settings = new LinearLayout(this);
        cameraChoice = selector(settings, "Camera");
        resolutionChoice = selector(settings, "Resolution");
        fpsChoice = selector(settings, "FPS target");
        qualityChoice = selector(settings, "Image quality");
        setItems(qualityChoice, Arrays.asList("Economy", "Balanced", "High"));
        qualityChoice.setSelection(1);
        cameraChoice.setOnItemSelectedListener(selection(this::refreshResolutions));
        resolutionChoice.setOnItemSelectedListener(selection(this::refreshFps));
        content.addView(settings);
        LinearLayout buttons = new LinearLayout(this);
        start = new Button(this);
        start.setText("Start");
        start.setOnClickListener(view -> requestStart());
        buttons.addView(start);
        stop = new Button(this);
        stop.setText("Stop");
        stop.setEnabled(false);
        stop.setOnClickListener(view -> stopSession());
        buttons.addView(stop);
        content.addView(buttons);
        status = new TextView(this);
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        status.setText("Stopped");
        content.addView(status);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(content);
        setContentView(scroll);
        loadCameras();
    }

    private Spinner selector(LinearLayout row, String label) {
        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        TextView heading = new TextView(this);
        heading.setText(label);
        column.addView(heading);
        Spinner spinner = new Spinner(this);
        spinner.setContentDescription(label);
        column.addView(spinner);
        row.addView(column, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        return spinner;
    }

    private <T> void setItems(Spinner spinner, List<T> items) {
        ArrayAdapter<T> adapter = new ArrayAdapter<>(this, android.R.layout.simple_spinner_item, items);
        adapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        spinner.setAdapter(adapter);
    }

    private AdapterView.OnItemSelectedListener selection(Runnable changed) {
        return new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, View view, int position, long id) { changed.run(); }
            @Override public void onNothingSelected(AdapterView<?> parent) {}
        };
    }

    private void loadCameras() {
        try {
            List<CameraOptions.Camera> cameras = CameraOptions.enumerate(getSystemService(CameraManager.class));
            setItems(cameraChoice, cameras);
            if (cameras.isEmpty()) status.setText("No supported camera modes. Grant camera permission with Start and try again.");
            refreshResolutions();
        } catch (Exception error) {
            status.setText("Could not list cameras. Grant camera permission with Start and try again.");
        }
    }

    private void refreshResolutions() {
        CameraOptions.Camera camera = (CameraOptions.Camera) cameraChoice.getSelectedItem();
        List<Size> sizes = new ArrayList<>();
        if (camera != null) {
            for (CameraOptions.Mode mode : camera.modes) {
                if (!sizes.contains(mode.size)) sizes.add(mode.size);
            }
        }
        Size previous = (Size) resolutionChoice.getSelectedItem();
        setItems(resolutionChoice, sizes);
        int selected = sizes.indexOf(previous);
        if (selected >= 0) resolutionChoice.setSelection(selected);
        refreshFps();
    }

    private void refreshFps() {
        CameraOptions.Camera camera = (CameraOptions.Camera) cameraChoice.getSelectedItem();
        Size size = (Size) resolutionChoice.getSelectedItem();
        Integer previous = (Integer) fpsChoice.getSelectedItem();
        List<Integer> rates = new ArrayList<>();
        if (camera != null && size != null) {
            for (CameraOptions.Mode mode : camera.modes) {
                if (mode.size.equals(size)) rates.add(mode.fps);
            }
        }
        setItems(fpsChoice, rates);
        int selected = rates.indexOf(previous);
        if (selected >= 0) fpsChoice.setSelection(selected);
    }

    private void requestStart() {
        if (current != null) return;
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] { Manifest.permission.CAMERA }, CAMERA_PERMISSION);
        } else {
            startSession();
        }
    }

    @Override public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        if (requestCode != CAMERA_PERMISSION || isFinishing()) return;
        if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) {
            if (cameraChoice.getCount() == 0) {
                loadCameras();
                status.setText("Select video settings, then tap Start.");
            } else {
                startSession();
            }
        } else {
            status.setText("Camera permission is required.");
        }
    }

    private void startSession() {
        if (current != null) return;
        try {
            CameraOptions.Camera camera = (CameraOptions.Camera) cameraChoice.getSelectedItem();
            Size size = (Size) resolutionChoice.getSelectedItem();
            Integer fps = (Integer) fpsChoice.getSelectedItem();
            if (camera == null || size == null || fps == null) throw new IOException("No supported camera mode is selected.");
            CameraOptions.Mode selected = null;
            for (CameraOptions.Mode mode : camera.modes) {
                if (mode.size.equals(size) && mode.fps == fps) { selected = mode; break; }
            }
            if (selected == null) throw new IOException("Choose a supported frame rate.");
            CaptureSettings settings = new CaptureSettings(size.getWidth(), size.getHeight(), fps,
                    QUALITIES[qualityChoice.getSelectedItemPosition()], camera.front);
            Connection.Invite invite = Connection.Invite.parse(invitation.getText().toString());
            invitation.setText("");
            current = new Session(invite, camera, selected, settings);
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            controls(false);
            status.setText("Pairing with the PC…");
            current.start();
        } catch (IOException error) {
            status.setText(error.getMessage());
        }
    }

    private void controls(boolean enabled) {
        start.setEnabled(enabled);
        stop.setEnabled(!enabled);
        invitation.setEnabled(enabled);
        for (Spinner spinner : new Spinner[] { cameraChoice, resolutionChoice, fpsChoice, qualityChoice }) {
            spinner.setEnabled(enabled);
        }
    }

    private void stopSession() {
        if (current != null) {
            stop.setEnabled(false);
            status.setText("Stopping…");
            current.requestStop();
        }
    }

    private void finishSession(Session session, String message) {
        if (current != session) return;
        current = null;
        getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        controls(true);
        status.setText(message);
    }

    @Override protected void onStop() { stopSession(); super.onStop(); }

    private final class Session {
        private final Connection.Invite invite;
        private final CaptureSettings settings;
        private final Socket socket = new Socket();
        private final AtomicBoolean stopping = new AtomicBoolean();
        private final AtomicReference<String> cameraError = new AtomicReference<>();
        private final ArrayBlockingQueue<byte[]> frames = new ArrayBlockingQueue<>(1);
        private final PhoneCamera camera;
        private boolean paired;

        Session(Connection.Invite invite, CameraOptions.Camera selectedCamera, CameraOptions.Mode mode, CaptureSettings settings) {
            this.invite = invite;
            this.settings = settings;
            camera = new PhoneCamera(getSystemService(CameraManager.class), selectedCamera, mode, settings, frame -> {
                if (stopping.get()) return;
                frames.poll();
                frames.offer(frame);
            }, reason -> { cameraError.set(reason); requestStop(); });
        }

        void start() { new Thread(this::send, "Droid Webcam network").start(); }

        void requestStop() {
            if (!stopping.compareAndSet(false, true)) return;
            camera.stop();
            frames.clear();
            frames.offer(new byte[0]);
            ui.postDelayed(this::closeSocket, 300);
        }

        private void send() {
            String result = "Stopped. Use a new invitation to connect again.";
            try (Connection connection = Connection.open(socket, invite, settings)) {
                paired = true;
                Arrays.fill(invite.secret, (byte) 0);
                if (!stopping.get()) {
                    ui.post(() -> { if (current == this) status.setText("Paired. Opening selected camera…"); });
                    camera.start();
                }
                boolean first = true;
                long intervalStart = 0;
                int sent = 0;
                while (!stopping.get()) {
                    byte[] frame = frames.poll(first ? 10 : 3, TimeUnit.SECONDS);
                    if (frame == null) throw new IOException("Camera timed out.");
                    if (stopping.get()) break;
                    long now = System.nanoTime();
                    if (first) intervalStart = now;
                    connection.send(frame);
                    sent++;
                    long completed = System.nanoTime();
                    if (first || completed - intervalStart >= 1_000_000_000L) {
                        String measured = first ? "starting" : String.format(Locale.US, "%.1f fps", sent * 1_000_000_000.0 / (completed - intervalStart));
                        String message = (settings.front ? "Front" : "Rear") + " · " + settings.width + "×" + settings.height
                                + " · " + measured + " / " + settings.fps + " target";
                        ui.post(() -> { if (current == this && !stopping.get()) status.setText(message); });
                        first = false;
                        sent = 0;
                        intervalStart = completed;
                    }
                }
                connection.stop();
            } catch (SSLHandshakeException error) {
                if (!stopping.get()) result = "Host identity could not be verified. Copy a new invitation from the PC.";
            } catch (Exception error) {
                if (!stopping.get()) result = (paired ? "Disconnected. " : "Pairing failed. ")
                        + "Check Wi-Fi and the host, then start a new session.";
            } finally {
                stopping.set(true);
                Arrays.fill(invite.secret, (byte) 0);
                camera.stop();
                closeSocket();
                String reason = cameraError.get();
                String message = reason == null ? result : reason;
                ui.post(() -> finishSession(this, message));
            }
        }

        private void closeSocket() { try { socket.close(); } catch (IOException ignored) {} }
    }
}
