#!/usr/bin/env python3
"""Validate the local Android identity and upload it through authenticated GitHub CLI."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = 'siarheisinelnikau/droid-webcam'


def gh(*arguments, data=None):
    result = subprocess.run(['gh', *arguments, '--repo', REPOSITORY], input=data, capture_output=True)
    if result.returncode:
        raise SystemExit('GitHub CLI failed; secret values suppressed. Check gh auth status and environment access.')
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Validate locally without network or uploading')
    parser.add_argument('--replace', action='store_true', help='Allow updating existing environment secrets')
    args = parser.parse_args()
    keystore = ROOT / '.release-private/android-release.p12'
    password = (ROOT / '.release-private/android-keystore-password.txt').read_bytes().strip()
    if not password:
        raise SystemExit('The keystore password is empty.')
    result = subprocess.run(['openssl', 'pkcs12', '-in', str(keystore), '-clcerts', '-nokeys',
                             '-passin', 'env:LOCAL_KEYSTORE_PASSWORD'],
                            env=dict(os.environ, LOCAL_KEYSTORE_PASSWORD=password.decode()), capture_output=True)
    if result.returncode:
        raise SystemExit('Unable to unlock the keystore; sensitive output suppressed.')
    certificate = subprocess.run(['openssl', 'x509', '-outform', 'DER'], input=result.stdout, capture_output=True)
    expected = (ROOT / 'release/android-signing-cert.sha256').read_text().strip()
    if certificate.returncode or hashlib.sha256(certificate.stdout).hexdigest() != expected:
        raise SystemExit('Keystore certificate does not match the public digest in this repository.')
    print('Local release certificate matches the pinned digest.')
    if args.check:
        return
    if not shutil.which('gh'):
        raise SystemExit('Install GitHub CLI and run gh auth login, then run this command again.')
    values = {'ANDROID_KEYSTORE_BASE64': base64.b64encode(keystore.read_bytes()),
              'ANDROID_KEYSTORE_PASSWORD': password, 'ANDROID_KEY_PASSWORD': password}
    existing = {item['name'] for item in json.loads(gh('secret', 'list', '--env', 'release', '--json', 'name'))}
    if existing.intersection(values) and not args.replace:
        raise SystemExit('Signing secrets already exist. Use --replace only after checking this local identity.')
    for name, value in values.items():
        gh('secret', 'set', name, '--env', 'release', data=value)
        print('Uploaded ' + name + ' to ' + REPOSITORY + ' / release')
    saved = {item['name'] for item in json.loads(gh('secret', 'list', '--env', 'release', '--json', 'name'))}
    if not values.keys() <= saved:
        raise SystemExit('The environment is missing signing secrets; no release should be started.')
    print('Verified all three secret names. Values were sent through stdin and were not printed.')


if __name__ == '__main__':
    main()
