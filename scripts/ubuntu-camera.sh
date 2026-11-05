#!/usr/bin/env bash
# Manage only the explicitly named Droid Webcam virtual device.
set -euo pipefail
command_name="${1:-}"
device="${2:-/dev/video10}"
if [[ ! "$device" =~ ^/dev/video[0-9]+$ ]]; then
    echo 'Choose a device such as /dev/video10.' >&2
    exit 1
fi
if [[ "$command_name" != 'create' && "$command_name" != 'remove' ]]; then
    echo 'Usage: sudo ./scripts/ubuntu-camera.sh create|remove [/dev/video10]' >&2
    exit 1
fi
if (( EUID != 0 )); then
    echo 'Run this setup command with sudo.' >&2
    exit 1
fi
if ! command -v v4l2loopback-ctl >/dev/null; then
    echo 'Install v4l2loopback-dkms, v4l2loopback-utils and v4l-utils first. See README.md.' >&2
    exit 1
fi
name="/sys/class/video4linux/${device##*/}/name"
case "$command_name" in
    create)
        if [[ -e "$device" ]]; then
            echo "$device already exists. Choose an unused video number." >&2
            exit 1
        fi
        # Loading with zero devices leaves existing camera configuration intact.
        if [[ ! -d /sys/module/v4l2loopback ]]; then
            modprobe v4l2loopback devices=0
        fi
        v4l2loopback-ctl add -n 'Droid Webcam' -x 1 -b 2 -w 1920 -h 1080 "$device"
        if ! v4l2-ctl -d "$device" -c timeout=400; then
            v4l2loopback-ctl delete "$device"
            exit 1
        fi
        echo "Created $device. Run the host as a user with read/write access to this device."
        ;;
    remove)
        if [[ ! -f "$name" ]] || [[ "$(cat "$name")" != 'Droid Webcam' ]]; then
            echo 'This device is not labelled Droid Webcam.' >&2
            exit 1
        fi
        v4l2loopback-ctl delete "$device"
        echo "Removed $device."
        ;;
esac
