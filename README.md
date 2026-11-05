# Droid Webcam

Stream the rear camera of one Android phone over your home Wi-Fi to a Windows 11 camera source. Windows Camera Frame Server exposes **Droid Webcam** as a selectable video capture device. The prototype has one mode, 640 × 480 at a nominal 15 fps, and uses a direct connection on the local network.

The desktop receiver and Windows Media Foundation source are Rust. The Android app uses Camera2, Android’s JPEG encoder, and Java TLS. It captures while the app is open.

## Build

### Windows host

Use Windows 11 x64 (build 22000 or later), stable Rust with the `x86_64-pc-windows-msvc` target, Visual Studio Build Tools with the C++ workload, and a Windows 11 SDK. From PowerShell in the project directory, run

```powershell
rustup target add x86_64-pc-windows-msvc
.\build-windows.ps1
```

The script embeds the Rust COM media-source DLL in `dist/DroidWebcam.exe`. You can distribute the EXE by itself. Camera Frame Server needs the DLL at a stable location, so setup extracts it to `%ProgramFiles%\Droid Webcam` and registers its class under `HKLM\SOFTWARE\Classes\CLSID`. This follows the [Microsoft Virtual Camera sample](https://github.com/microsoft/Windows-Camera/tree/master/Samples/VirtualCamera) and the [Frame Server custom media source guide](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source).

### Android app

Use Android 10 or later on the phone. The build uses Bash, a JDK (tested with 21), `zip`, Android SDK Platform 35, and Android Build Tools 35.0.0. Set `ANDROID_HOME` to your SDK directory and, if needed, `JAVA_HOME` to the JDK.

```sh
sdkmanager 'platforms;android-35' 'build-tools;35.0.0'
./android/build.sh
```

The script creates a local debug keystore and signs `android/build/DroidWebcam-debug.apk`. Install it on your test phone after authorizing USB debugging, or transfer the APK for manual installation.

```sh
adb install -r android/build/DroidWebcam-debug.apk
```

## Run on Windows and Android

1. Open PowerShell as administrator in the directory containing the EXE and run `.\DroidWebcam.exe install-source` once. This is the machine-wide COM installation. Run the remaining camera commands in a regular PowerShell window.
2. Put the phone and PC on the same LAN. Find the PC’s IPv4 address with `ipconfig`. In the example below, replace `192.168.1.42` with that address.

   ```powershell
   .\DroidWebcam.exe serve 192.168.1.42:47470 --camera
   ```

   Allow the EXE’s inbound TCP port on your private network if Windows Firewall blocks the connection. Make sure Windows camera privacy settings allow camera access for your client application.
3. Copy the complete one-time invitation from PowerShell to the app using a transfer method you trust. Treat it as a password. Within three minutes, tap **Start** and grant camera permission if prompted. The Android app checks the host certificate before sending the secret and opens the rear camera after the host accepts it.
4. Select **Droid Webcam** in Windows Camera, Zoom, or Discord. In OBS, add a **Video Capture Device** and select **Droid Webcam**. Windows may append “Windows Virtual Camera” to the name. Position the phone in landscape and rotate it if the picture is upside down.
5. Tap **Stop** on Android or press **Enter** on the PC. Leaving the Android app also stops capture. A lost connection ends the session. Run `serve` again for a new invitation.

The virtual camera has current-user session lifetime and is removed when the host shuts it down. To remove the installed component, stop the host and run `.\DroidWebcam.exe uninstall-source` from elevated PowerShell. If Frame Server still holds the DLL open, reboot and repeat the removal command to delete the file. See [MFCreateVirtualCamera](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera) and [IMFVirtualCamera::Shutdown](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-imfvirtualcamera-shutdown).

## Checks

### Local tests without devices

```sh
cargo test --locked --workspace
cargo build --locked --release
python3 android/tests/loopback.py target/release/droid-webcam
```

The Rust tests cover pinned TLS, invalid credentials, invitation reuse, oversized and truncated input, JPEG validation, stop, disconnect, and stale-frame eviction. The Java test runs the app’s transport against the Rust receiver on loopback, including rejected credentials and an acknowledged JPEG. Both network tests need permission to open loopback TCP sockets.

For a manual receiver check, run `droid-webcam serve 127.0.0.1:47470` in one terminal and `droid-webcam send-test` in another, then paste the invitation into the second terminal. The sender sends moving color bars and an explicit stop. You can also run the receiver with `--camera` on Windows and use this sender in place of the phone.

### Device acceptance on Windows 11

1. After explicitly installing the source, run `.\DroidWebcam.exe test-pattern`. Open Windows Camera, select **Droid Webcam**, and confirm that the marker moves across the color bars. Repeat with an app you use.
2. Press **Enter** and confirm the camera disappears after refreshing the application’s device list. Start a LAN session and confirm that a real Android feed appears. Test Stop, leaving the app, disconnecting Wi-Fi, and pairing again with a fresh code.
3. Run the elevated removal command and confirm the device is gone and the COM component is removed.

The Windows x64 release was cross-built on Ubuntu with the Rust GNU target and checked for the embedded DLL, the Media Foundation import, and the COM exports. The Android debug APK was built against SDK Platform 35 and its signature was verified. Registration, application discovery, and rear-camera capture still need the Windows 11 and Android acceptance run above.

## How frames move

The invitation carries a host IPv4 endpoint, the SHA-256 fingerprint of a fresh host certificate, and a random 256-bit secret. Over TLS 1.3 the client sends `DW01` and the secret. The receiver consumes the invitation and returns `DWOK` before accepting frames. Each frame begins with its big-endian 32-bit byte length and contains a sequential color JPEG of at most 256 KiB. A zero length ends the session. The receiver acknowledges each decoded frame with byte `01` before the client sends another.

Android keeps one pending frame. The host keeps the most recent decoded NV12 frame and hands it to Frame Server over a local, one-way named pipe. The COM media source discards frames older than 400 ms and provides black frames while waiting for video. The source publishes a single uncompressed Media Foundation video stream.

A browser would need a trusted HTTPS origin to request the camera from a phone at a LAN address. The APK uses Android’s camera permission and pins the host certificate itself. See the [Media Capture and Streams specification](https://w3c.github.io/mediacapture-main/) and the [Secure Contexts specification](https://w3c.github.io/webappsec-secure-contexts/).

## License

MIT. See [LICENSE](LICENSE).
