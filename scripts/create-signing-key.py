#!/usr/bin/env python3
"""Create the initial Android release identity once; never replace an existing key."""
import hashlib
import os
from pathlib import Path
import secrets
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PRIVATE = ROOT / '.release-private'
KEYSTORE = PRIVATE / 'android-release.p12'
PASSWORD = PRIVATE / 'android-keystore-password.txt'
PIN = ROOT / 'release/android-signing-cert.sha256'
CERTIFICATE = ROOT / 'release/android-signing-cert.pem'


def main():
    if any(path.exists() for path in (KEYSTORE, PASSWORD, PIN, CERTIFICATE)):
        raise SystemExit('A release identity already exists. Refusing to rotate or overwrite it.')
    PRIVATE.mkdir(mode=0o700, exist_ok=True)
    PRIVATE.chmod(0o700)
    secret = secrets.token_urlsafe(48)
    env = dict(os.environ, NEW_SIGNING_PASSWORD=secret)
    with tempfile.TemporaryDirectory(prefix='create-', dir=PRIVATE) as directory:
        work = Path(directory)
        commands = [
            ['openssl', 'req', '-new', '-newkey', 'rsa:3072', '-x509', '-sha256', '-days', '10000',
             '-subj', '/CN=Droid Webcam Release/O=Droid Webcam', '-keyout', str(work / 'key.pem'),
             '-out', str(work / 'cert.pem'), '-passout', 'env:NEW_SIGNING_PASSWORD'],
            ['openssl', 'pkcs12', '-export', '-inkey', str(work / 'key.pem'), '-in', str(work / 'cert.pem'),
             '-name', 'droid-webcam-release', '-out', str(work / 'release.p12'),
             '-passin', 'env:NEW_SIGNING_PASSWORD', '-passout', 'env:NEW_SIGNING_PASSWORD'],
        ]
        for command in commands:
            result = subprocess.run(command, env=env, capture_output=True)
            if result.returncode:
                raise SystemExit('OpenSSL could not create the release identity; sensitive output suppressed.')
        certificate = subprocess.check_output(['openssl', 'x509', '-in', str(work / 'cert.pem'), '-outform', 'DER'])
        for destination, data in ((KEYSTORE, (work / 'release.p12').read_bytes()), (PASSWORD, secret.encode())):
            with os.fdopen(os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as output:
                output.write(data)
        PIN.parent.mkdir(exist_ok=True)
        with PIN.open('x') as output:
            output.write(hashlib.sha256(certificate).hexdigest() + '\n')
        with CERTIFICATE.open('xb') as output:
            output.write((work / 'cert.pem').read_bytes())
    print('Created the encrypted release keystore and local password with owner-only permissions.')
    print('Only the public certificate and its SHA-256 digest belong in Git. Back up the private files separately.')


if __name__ == '__main__':
    main()
