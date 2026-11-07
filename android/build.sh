#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
mode="${1:-debug}"
case "$mode" in
    debug|release|unsigned) ;;
    *) printf '%s\n' 'Usage: android/build.sh [debug|release|unsigned]' >&2; exit 1 ;;
esac

sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
: "${sdk:?Set ANDROID_HOME to your Android SDK directory}"
tools="$sdk/build-tools/35.0.0"
platform="$sdk/platforms/android-35/android.jar"
if [[ ! -f "$platform" || ! -x "$tools/apksigner" ]]; then
    printf '%s\n' 'Install platforms;android-35 and build-tools;35.0.0 using sdkmanager.' >&2
    exit 1
fi
if [[ -n "${JAVA_HOME:-}" ]]; then export PATH="$JAVA_HOME/bin:$PATH"; fi

mkdir -p build/classes build/dex
javac --release 8 -Xlint:-options -classpath "$platform" -d build/classes src/com/droidwebcam/*.java
jar cf build/classes.jar -C build/classes .
"$tools/d8" --min-api 29 --lib "$platform" --output build/dex build/classes.jar
"$tools/aapt2" link -I "$platform" --manifest AndroidManifest.xml -o build/resources.apk
cp build/resources.apk build/unsigned.apk
(cd build/dex && zip -q ../unsigned.apk classes.dex)
"$tools/zipalign" -f 4 build/unsigned.apk build/aligned.apk
if [[ "$mode" == unsigned ]]; then
    printf '%s\n' "Built unsigned $PWD/build/aligned.apk"
    exit 0
fi
./sign-apk.sh "$mode" build/aligned.apk "build/DroidWebcam-$mode.apk"
