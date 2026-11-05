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
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.IOException;
import java.net.Socket;
import java.util.Arrays;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import javax.net.ssl.SSLHandshakeException;

public final class MainActivity extends Activity {
    private static final int CAMERA_PERMISSION = 1;
    private final Handler ui = new Handler(Looper.getMainLooper());
    private EditText invitation;
    private TextView status;
    private Button start;
    private Button stop;
    private Session current;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        int padding = Math.round(24 * getResources().getDisplayMetrics().density);
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
        instructions.setText("Join the PC’s Wi-Fi. Paste its one-time invitation, then tap Start to approve pairing and use the rear camera.");
        content.addView(instructions);
        invitation = new EditText(this);
        invitation.setHint("One-time invitation");
        invitation.setSingleLine();
        invitation.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        invitation.setImeOptions(EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        invitation.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
        invitation.setSaveEnabled(false);
        invitation.setFilters(new InputFilter[] { new InputFilter.LengthFilter(192) });
        content.addView(invitation);
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
            startSession();
        } else {
            status.setText("Camera permission is required.");
        }
    }

    private void startSession() {
        if (current != null) return;
        try {
            Connection.Invite invite = Connection.Invite.parse(invitation.getText().toString());
            invitation.setText("");
            current = new Session(invite);
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            start.setEnabled(false);
            stop.setEnabled(true);
            status.setText("Pairing with the PC…");
            current.start();
        } catch (IOException error) {
            status.setText(error.getMessage());
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
        start.setEnabled(true);
        stop.setEnabled(false);
        status.setText(message);
    }

    @Override protected void onStop() {
        stopSession();
        super.onStop();
    }

    private final class Session {
        private final Connection.Invite invite;
        private final Socket socket = new Socket();
        private final AtomicBoolean stopping = new AtomicBoolean();
        private final AtomicReference<String> cameraError = new AtomicReference<>();
        private final ArrayBlockingQueue<byte[]> frames = new ArrayBlockingQueue<>(1);
        private final RearCamera camera;
        private boolean paired;

        Session(Connection.Invite invite) {
            this.invite = invite;
            camera = new RearCamera(getSystemService(CameraManager.class), frame -> {
                if (stopping.get()) return;
                frames.poll();
                frames.offer(frame);
            }, reason -> {
                cameraError.set(reason);
                requestStop();
            });
        }

        void start() { new Thread(this::send, "Droid Webcam network").start(); }

        void requestStop() {
            if (!stopping.compareAndSet(false, true)) return;
            camera.stop();
            frames.clear();
            frames.offer(new byte[0]); // Wake a sender waiting for a camera frame.
            ui.postDelayed(this::closeSocket, 300);
        }

        private void send() {
            String result = "Stopped. Use a new invitation to connect again.";
            try (Connection connection = Connection.open(socket, invite)) {
                paired = true;
                Arrays.fill(invite.secret, (byte) 0);
                if (!stopping.get()) {
                    ui.post(() -> { if (current == this) status.setText("Paired. Opening rear camera…"); });
                    camera.start();
                }
                boolean first = true;
                while (!stopping.get()) {
                    byte[] frame = frames.poll(first ? 10 : 3, TimeUnit.SECONDS);
                    if (frame == null) throw new IOException("Camera timed out.");
                    if (stopping.get()) break;
                    connection.send(frame);
                    if (first) {
                        first = false;
                        ui.post(() -> { if (current == this) status.setText("Streaming rear camera · 640×480"); });
                    }
                }
                connection.stop();
            } catch (SSLHandshakeException error) {
                if (!stopping.get()) result = "Host identity could not be verified. Copy a new invitation from the PC.";
            } catch (Exception error) {
                if (!stopping.get()) {
                    result = (paired ? "Disconnected. " : "Pairing failed. ")
                            + "Check Wi-Fi and start a new session on the PC.";
                }
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

        private void closeSocket() {
            try { socket.close(); } catch (IOException ignored) {}
        }
    }
}
