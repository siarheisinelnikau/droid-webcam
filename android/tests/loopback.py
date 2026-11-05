#!/usr/bin/env python3
"""Run the Android Java transport against the Rust receiver in every video mode."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    android = Path(__file__).resolve().parents[1]
    host = Path(sys.argv[1]).resolve()
    java_bin = Path(os.environ["JAVA_HOME"]) / "bin" if "JAVA_HOME" in os.environ else None

    def java(command):
        return str(java_bin / command) if java_bin is not None else command

    with tempfile.TemporaryDirectory() as classes:
        subprocess.run(
            [java("javac"), "--release", "8", "-Xlint:-options", "-d", classes,
             android / "src/com/droidwebcam/Connection.java",
             android / "src/com/droidwebcam/CaptureSettings.java", android / "tests/LoopbackClient.java"],
            check=True,
        )
        cases = [[str(width), str(height), str(fps), str(quality), str(front).lower()]
                 for width, height in [(640, 480), (1280, 720), (1920, 1080)]
                 for fps, quality, front in [(15, 55, False), (30, 80, True), (60, 95, False)]]
        cases.append(["legacy"])
        for case in cases:
            with subprocess.Popen(
                [host, "serve", "127.0.0.1:0"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
            ) as receiver:
                try:
                    receiver.stdout.readline()
                    invitation = receiver.stdout.readline().strip()
                    if not invitation.startswith("droidcam:2:"):
                        raise AssertionError("Receiver did not print an invitation")
                    subprocess.run(
                        [java("java"), "-Djava.awt.headless=true", "-cp", classes,
                         "com.droidwebcam.LoopbackClient", *case],
                        input=invitation + "\n", text=True, check=True, timeout=35,
                    )
                    output, _ = receiver.communicate(timeout=5)
                    if receiver.returncode != 0 or "Streaming." not in output or "Stopped." not in output:
                        raise AssertionError(output)
                    if case != ["legacy"]:
                        expected = f"{case[0]}x{case[1]}@{case[2]}, quality {case[3]}"
                        lens = "front camera" if case[4] == "true" else "rear camera"
                        if expected not in output or lens not in output:
                            raise AssertionError(output)
                finally:
                    if receiver.poll() is None:
                        receiver.kill()
            print("Java TLS loopback passed", " ".join(case), flush=True)


if __name__ == "__main__":
    main()
