# Droid Webcam

Use an Android phone as a camera on **Windows 11 or Ubuntu** over your home Wi-Fi. The Rust host publishes video through Windows Media Foundation or an Ubuntu V4L2 loopback device. The Android app provides explicit **Start** and **Stop** controls and lets you choose the camera and capture settings before starting a session.

- Front or rear camera. Phones with multiple camera IDs show each supported choice.
- 640 × 480, 1280 × 720, or 1920 × 1080.
- A target of 15, 30, or 60 FPS. The app offers combinations supported by that camera’s normal YUV capture and automatic exposure metadata.
- Economy, Balanced, or High image quality, using JPEG quality 55, 80, or 95.

The default is **720p at 30 FPS with Balanced quality**, when supported by the phone. The status line shows the actual rate acknowledged by the host alongside your target. Achievable FPS depends on the phone, JPEG encoding, host decoding, and Wi-Fi throughput. The app keeps one pending frame to avoid accumulating delay.

The connection runs directly on the LAN with a single-use invitation and pinned TLS 1.3. The desktop receiver and both camera backends are Rust. Android uses a small Java layer for Camera2, JPEG encoding, and platform TLS.

## GitHub releases

The [Release workflow](https://github.com/siarheisinelnikau/droid-webcam/actions/workflows/release.yml) builds Windows x64, Ubuntu x64, and Android downloads on GitHub-hosted runners. It runs Rust tests and the Java/Rust transport checks, generates SHA-256 checksums, and publishes a preview release after all jobs pass.

Run it manually from **Actions → Release → Run workflow** with a tag matching the Rust and Android package version, such as `v0.2.0`. A manual run builds the selected workflow ref and creates that tag after the build succeeds. Pushing a `v*` tag also starts a release build. Add matching notes under `docs/releases/<tag>.md` before starting. Existing published releases and tags pointing to a different commit are preserved.

Release APKs use a permanent certificate and the `release` GitHub environment. Build jobs do not receive signing credentials. The signing job checks the public certificate digest in `release/android-signing-cert.sha256` and stops if credentials are missing or the certificate changes. See [release signing and backup](docs/signing.md) for initial setup and recovery.

Switching from an older debug-signed APK to the first release-signed APK requires uninstalling the older app, which removes its app data. Later release APKs can update the installed release app with the same certificate and a nondecreasing Android version code. Do not replace an existing published release to make this transition.

## Build

### Windows

Use Windows 11 x64 (build 22000 or later), stable Rust, Visual Studio Build Tools with the C++ workload, and a Windows 11 SDK. In PowerShell, run

```powershell
rustup target add x86_64-pc-windows-msvc
.\build-windows.ps1
```

The script embeds the COM media-source DLL in `dist/DroidWebcam.exe`. Distribute that EXE. The explicit `install-source` command extracts the DLL to `%ProgramFiles%\Droid Webcam` and registers its class under `HKLM\SOFTWARE\Classes\CLSID`. Camera Frame Server loads this registered component. This follows the official virtual camera sample and custom media source guidance. [Microsoft sample](https://github.com/microsoft/Windows-Camera/tree/master/Samples/VirtualCamera), [Frame Server guide](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source).

### Ubuntu

Use stable Rust and a C compiler. Build on the Ubuntu system you intend to run the host on, or use the supplied x86-64 Linux binary with glibc 2.34 or later and libgcc_s.

```sh
./build-ubuntu.sh
```

The output is `dist/droid-webcam`. The camera backend uses the V4L2 API directly. Install the loopback driver and utilities, then explicitly create a virtual camera at an unused video number.

```sh
sudo apt install build-essential linux-headers-$(uname -r) v4l2loopback-dkms v4l2loopback-utils v4l-utils
sudo ./scripts/ubuntu-camera.sh create /dev/video10
```

The setup script loads the module with zero initial devices when needed, creates the chosen device with the label **Droid Webcam**, enables exclusive capture/output capabilities, and sets a 400 ms blank-frame timeout. The device becomes discoverable to camera applications after a producer starts. Driver installation may require the module-signing steps prompted by Ubuntu when Secure Boot is enabled. [v4l2loopback documentation](https://github.com/v4l2loopback/v4l2loopback).

Run the host as your regular desktop user with read/write access to the device. If your session has no device ACL, membership in the `video` group provides access. Close the host and camera applications before removing this device.

```sh
sudo ./scripts/ubuntu-camera.sh remove /dev/video10
```

Removal checks the device label and deletes that individual loopback device. Device creation is temporary and can be repeated after reboot.

### Android

Use Android 10 or later. Building needs Bash, a JDK (tested with 21), `zip`, Android SDK Platform 35, and Android Build Tools 35.0.0. Set `ANDROID_HOME` and, if needed, `JAVA_HOME`.

```sh
sdkmanager 'platforms;android-35' 'build-tools;35.0.0'
./android/build.sh
adb install -r android/build/DroidWebcam-debug.apk
```

The script creates a local debug keystore and signs the APK. You can also transfer the APK to the phone for manual installation.

## Run

1. Put the phone and computer on the same LAN. Find the computer’s IPv4 address with `ipconfig` on Windows or `ip -4 addr` on Ubuntu.
2. On Windows, run `DroidWebcam.exe install-source` once in elevated PowerShell. Then start the host from regular PowerShell in the EXE directory, substituting your LAN address.

   ```powershell
   .\DroidWebcam.exe serve 192.168.1.42:47470 --camera
   ```

   On Ubuntu, create the loopback device as described above and start the host as your regular user.

   ```sh
   ./dist/droid-webcam serve 192.168.1.42:47470 --camera --device /dev/video10
   ```

   Allow inbound TCP port 47470 on your private network if the firewall blocks it. Windows camera privacy settings must allow your client application to access cameras.
3. In Android, select a camera, resolution, target FPS, and image quality. Copy the complete invitation from the host using a method you trust and tap **Start** within three minutes. Treat the invitation as a password. The app verifies the host certificate and negotiates settings before opening the camera.
4. Select **Droid Webcam** in your camera application. On Windows, the name may include “Windows Virtual Camera”. In OBS, add a **Video Capture Device**. On Ubuntu, refresh the list after the phone starts streaming. Position the phone in landscape.
5. Check the Android status line for the actual FPS. Tap **Stop**, leave the Android app, or press **Enter** on the host to end the session. Run the host again for a fresh invitation when changing settings or reconnecting.

The Windows source offers all nine output modes to applications, with 720p/30 as the default. An application can choose its own resolution and FPS. The source fits the phone image into that resolution with neutral borders when the aspect ratio differs. Choosing a higher output FPS repeats input frames when necessary. On Ubuntu, the V4L2 device uses the phone’s negotiated resolution and FPS for the session. Close camera clients before changing modes.

To remove the Windows component, stop the host and run `DroidWebcam.exe uninstall-source` from elevated PowerShell. If Frame Server retains the DLL, reboot and repeat the command. The virtual device uses current-user session lifetime and is removed when the host shuts it down. [MFCreateVirtualCamera](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera), [IMFVirtualCamera::Shutdown](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-imfvirtualcamera-shutdown).

When upgrading from the original build, remove the old Windows source first and install the new one. The host verifies that the installed DLL matches its embedded version before starting. Update the Android APK and host together to use the new controls.

## Checks

### Checks without camera devices

```sh
cargo test --locked --workspace
cargo build --locked --release
python3 android/tests/loopback.py target/release/droid-webcam
```

The tests cover mode negotiation, front/rear selection, quality bounds, pinned TLS, invalid credentials, invitation reuse, malformed settings, oversized or truncated records, JPEG resolution validation, stop, disconnect, stale frames, NV12 resizing, and Linux UAPI layouts. The Java loopback uses the app’s transport in all nine modes, including JPEGs above the original 256 KiB limit, and checks compatibility with the original protocol. Loopback tests need permission to open TCP sockets on `127.0.0.1`.

For a manual test, run a receiver without `--camera` and connect a synthetic sender in a second terminal. Paste the receiver’s invitation into the sender.

```sh
droid-webcam serve 127.0.0.1:47470
droid-webcam send-test 300 --mode 1280x720@30 --quality 80
```

### Device acceptance

After explicitly installing or creating the virtual device, use the synthetic source first.

```powershell
.\DroidWebcam.exe test-pattern --mode 1280x720@30
```

```sh
./dist/droid-webcam test-pattern --device /dev/video10 --mode 1280x720@30
```

Select **Droid Webcam** in OBS or another camera application and confirm that the marker moves across the color bars. On Windows, select output modes such as 720p/60 and 1080p/30 in the application. Then test a real phone with each available camera, multiple capture modes, Stop, leaving the app, a Wi-Fi interruption, and a fresh pairing. Check removal after closing the host and clients.

The Linux host and Android debug APK have been built on Ubuntu. The Windows x64 EXE has been cross-built and inspected for the embedded DLL, Media Foundation import, and COM exports. The Android APK signature and Java/Rust transport have been checked. Physical Android capture, Windows application discovery, and Ubuntu streaming through a loaded `v4l2loopback` device still need the device acceptance run above.

## Protocol and bounds

A version 2 invitation carries an IPv4 endpoint, the SHA-256 fingerprint of a fresh host certificate, and a random 256-bit secret. Inside TLS 1.3 the client sends `DW02`, the secret, and eight bytes describing width, height, FPS, JPEG quality, and front/rear camera selection. The host validates and echoes the settings with `DWOK`. The invitation is consumed once the secret is authenticated, including if settings are subsequently rejected.

Frames use a big-endian 32-bit byte length and a sequential RGB JPEG matching the negotiated dimensions, bounded to 3 MiB. A zero length ends the session. The host acknowledges a decoded frame with byte `01`. The original `DW01` handshake remains accepted with 640×480/15 and its 256 KiB bound. Host decoding buffers are bounded by the negotiated resolution, and the Windows pipe has a versioned header and bounded NV12 payload.

The Android camera list checks normal AE FPS ranges and YUV minimum frame duration. High-speed capture available only through constrained sessions is excluded. Camera metadata describes the target’s availability, while the status line measures frames accepted by the host. [Camera characteristics](https://developer.android.com/reference/android/hardware/camera2/CameraCharacteristics), [Stream configuration](https://developer.android.com/reference/android/hardware/camera2/params/StreamConfigurationMap).

## License

MIT. See [LICENSE](LICENSE). The separately installed `v4l2loopback` driver has its own license.
