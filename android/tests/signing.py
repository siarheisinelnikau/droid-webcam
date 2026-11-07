#!/usr/bin/env python3
"""Credential handling and certificate rejection checks using a fake SDK signer."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='droid-signing-test-')
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        signer = self.work / 'sdk/build-tools/35.0.0/apksigner'
        signer.parent.mkdir(parents=True)
        signer.write_text('''#!/usr/bin/env python3
import os, pathlib, sys
args = sys.argv[1:]
if args[0] == 'sign':
    assert 'env:ANDROID_KEYSTORE_PASSWORD' in args
    assert 'env:ANDROID_KEY_PASSWORD' in args
    assert all(os.environ[name] not in args for name in ['ANDROID_KEYSTORE_PASSWORD','ANDROID_KEY_PASSWORD'])
    pathlib.Path(args[args.index('--out')+1]).write_bytes(b'fixture APK')
elif args[0] == 'verify':
    if os.environ.get('FAKE_VERIFY_FAILURE'):
        sys.exit(1)
    print('Signer #1 certificate SHA-256 digest: '+os.environ['FAKE_CERTIFICATE'])
else:
    sys.exit(1)
''')
        signer.chmod(0o755)
        (self.work / 'test.p12').write_bytes(b'not a production key')
        (self.work / 'aligned.apk').write_bytes(b'fixture input')
        self.output = self.work / 'signed.apk'
        self.env = {key: value for key, value in os.environ.items() if not key.startswith('ANDROID_')}
        self.env.update(ANDROID_HOME=str(self.work / 'sdk'), ANDROID_KEYSTORE_PATH=str(self.work / 'test.p12'),
                        ANDROID_KEYSTORE_PASSWORD='fixture-store-password', ANDROID_KEY_PASSWORD='fixture-key-password',
                        FAKE_CERTIFICATE=(ROOT / 'release/android-signing-cert.sha256').read_text().strip())

    def run_signer(self):
        return subprocess.run(['bash', str(ROOT / 'android/sign-apk.sh'), 'release',
                               str(self.work / 'aligned.apk'), str(self.output)],
                              env=self.env, capture_output=True, text=True)

    def test_passwords_use_environment_and_expected_certificate_is_accepted(self):
        result = self.run_signer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.output.is_file())
        self.assertNotIn('fixture-store-password', result.stdout + result.stderr)
        self.assertNotIn('fixture-key-password', result.stdout + result.stderr)

    def test_missing_credentials_never_fall_back_to_debug_signing(self):
        del self.env['ANDROID_KEYSTORE_PASSWORD']
        result = self.run_signer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())

    def test_unexpected_certificate_removes_the_signed_output(self):
        self.env['FAKE_CERTIFICATE'] = '0' * 64
        result = self.run_signer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('does not match', result.stderr)
        self.assertFalse(self.output.exists())

    def test_invalid_signature_removes_the_signed_output(self):
        self.env['FAKE_VERIFY_FAILURE'] = '1'
        result = self.run_signer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
