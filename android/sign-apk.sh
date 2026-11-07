#!/usr/bin/env bash
set -euo pipefail
set +x
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
mode="${1:?Usage: sign-apk.sh debug|release INPUT.apk OUTPUT.apk}"
input="${2:?Provide the aligned input APK}"
output="${3:?Provide the output APK}"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
: "${sdk:?Set ANDROID_HOME}"
signer="$sdk/build-tools/35.0.0/apksigner"
if [[ -n "${JAVA_HOME:-}" ]]; then export PATH="$JAVA_HOME/bin:$PATH"; fi

case "$mode" in
    release)
        : "${ANDROID_KEYSTORE_PATH:?Set ANDROID_KEYSTORE_PATH}"
        : "${ANDROID_KEYSTORE_PASSWORD:?Set ANDROID_KEYSTORE_PASSWORD}"
        : "${ANDROID_KEY_PASSWORD:?Set ANDROID_KEY_PASSWORD}"
        test -s "$ANDROID_KEYSTORE_PATH"
        expected="$(tr -d '\r\n' < "$root/release/android-signing-cert.sha256")"
        [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'Invalid pinned certificate digest.' >&2; exit 1; }
        "$signer" sign --ks "$ANDROID_KEYSTORE_PATH" --ks-type PKCS12 \
            --ks-key-alias droid-webcam-release --ks-pass env:ANDROID_KEYSTORE_PASSWORD \
            --key-pass env:ANDROID_KEY_PASSWORD --out "$output" "$input"
        ;;
    debug)
        keystore="$root/android/build/debug.keystore"
        if [[ ! -f "$keystore" ]]; then
            keytool -genkeypair -keystore "$keystore" -storepass android -keypass android \
                -alias androiddebugkey -dname 'CN=Android Debug,O=Droid Webcam' -keyalg RSA -keysize 2048 -validity 10000
        fi
        "$signer" sign --ks "$keystore" --ks-pass pass:android --key-pass pass:android \
            --out "$output" "$input"
        ;;
    *) echo 'Signing mode must be debug or release.' >&2; exit 1 ;;
esac

if ! verification="$("$signer" verify --verbose --print-certs "$output")"; then
    rm -f -- "$output" "$output.idsig"
    echo 'APK signature verification failed.' >&2
    exit 1
fi
if [[ "$mode" == release ]]; then
    actual="$(printf '%s\n' "$verification" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | tr '[:upper:]' '[:lower:]')"
    if [[ "$actual" != "$expected" ]]; then
        rm -f -- "$output" "$output.idsig"
        echo 'The APK certificate does not match the pinned release certificate.' >&2
        exit 1
    fi
fi
printf '%s\n' "Verified $mode APK: $output"
