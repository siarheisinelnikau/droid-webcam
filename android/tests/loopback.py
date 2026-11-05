#!/usr/bin/env python3
"""Exercise the Android client's actual Java transport against the Rust receiver."""
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
             android / "src/com/droidwebcam/Connection.java", android / "tests/LoopbackClient.java"],
            check=True,
        )
        with subprocess.Popen(
            [host, "serve", "127.0.0.1:0"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        ) as receiver:
            try:
                # The first two lines are the pairing prompt and invitation.
                receiver.stdout.readline()
                invitation = receiver.stdout.readline().strip()
                if not invitation.startswith("droidcam:1:"):
                    raise AssertionError("Receiver did not print an invitation")
                subprocess.run(
                    [java("java"), "-Djava.awt.headless=true", "-cp", classes, "com.droidwebcam.LoopbackClient"],
                    input=invitation + "\n", text=True, check=True, timeout=25,
                )
                output, _ = receiver.communicate(timeout=5)
                if receiver.returncode != 0 or "Streaming." not in output or "Stopped." not in output:
                    raise AssertionError(output)
            finally:
                if receiver.poll() is None:
                    receiver.kill()
    print("Java TLS loopback passed: wrong pin, wrong secret, JPEG, acknowledgement and stop")


if __name__ == "__main__":
    main()
