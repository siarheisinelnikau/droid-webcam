# Android release signing

GitHub release APKs use one persistent RSA-3072 release identity in a password-encrypted PKCS#12 keystore. Its public certificate is `release/android-signing-cert.pem`; its SHA-256 digest is pinned in `release/android-signing-cert.sha256`. These two public files can be committed. The private files under `.release-private/` must never be committed, attached to a release, or uploaded as an Actions artifact.

## GitHub setup

Use **Settings → Environments → release**, restricted to branch `main` and tags matching `v*`. The environment requires three separate secrets:

| Secret | Source |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | Base64 encoding of `.release-private/android-release.p12` |
| `ANDROID_KEYSTORE_PASSWORD` | `.release-private/android-keystore-password.txt` |
| `ANDROID_KEY_PASSWORD` | The same password: this PKCS#12 store uses one password for the store and key |

The alias is the non-sensitive constant `droid-webcam-release`; it is not another secret. Base64 encodes a binary file; it does not encrypt it. Encryption comes from the keystore password and GitHub's Secrets service.

With Python 3, OpenSSL, and an authenticated [GitHub CLI](https://cli.github.com/) available, run from this checkout:

```sh
python3 scripts/upload-signing-secrets.py --check
python3 scripts/upload-signing-secrets.py
```

Use absolute script paths from another directory. The uploader unlocks the store, checks the public certificate against the committed digest, uploads values to `siarheisinelnikau/droid-webcam` / `release` through stdin, and verifies all three secret names. It does not print values or put passwords in command arguments. Existing signing secrets require explicit `--replace`; an interrupted three-secret upload can be retried with that flag after checking the same local identity.

On 2026-11-07, the encrypted keystore, local password file, public certificate, and restricted GitHub environment were created, and all three secrets were saved through the existing authenticated GitHub browser session. Their names were confirmed in the environment's secret list without displaying the values. The CLI uploader above remains an optional recovery/replacement path. The real SDK signing job still needs its first remote validation run; missing or mismatched credentials deliberately stop publication.

## Builds and updates

### Release from any computer

1. Commit a new version in both Rust package manifests and `Cargo.lock`, update Android `versionName`, increment Android `versionCode`, and add matching notes in `docs/releases/vVERSION.md`. Previously published versions and tags must not be reused.
2. Open **Actions → Release → Run workflow** in GitHub, select `main`, and enter the matching `vVERSION` tag. Repository Actions access is required; no local SDK, keystore, or CLI login is needed.
3. Leave **Publish a new version after build and signing checks** unchecked to build and sign all artifacts without creating a release. The Android APK is verified against the permanent certificate before upload as an artifact.
4. Enable that option to publish a prepared new version. The workflow builds Windows, Ubuntu, and Android packages, signs the APK, checks the artifacts, and publishes the preview release with checksums. A `v*` tag-push trigger retains the existing automatic-publication behavior.

`android/build.sh` defaults to local debug signing. `android/build.sh unsigned` compiles and aligns an APK without any signing credential. The GitHub build job uses this mode. A separate GitHub-hosted `android-sign` job declares the `release` environment, decodes the keystore into an owner-only temporary file, and calls `android/sign-apk.sh release INPUT.apk OUTPUT.apk`. Passwords reach `apksigner` through environment variables, not process arguments. The temporary keystore is removed on exit. Only the signed APK is passed to the release-publishing job.

The signer verifies both the APK signature and the committed certificate digest. Missing secrets, invalid signatures, and a different certificate stop publication; no debug fallback is permitted for releases. Signing follows zip alignment. Changes after signing would invalidate the APK signature.

For a local release build, set `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, and `ANDROID_KEY_PASSWORD` in a secure local session and run `android/build.sh release`. Never use shell tracing for those variables.

The first release APK has a different certificate from earlier debug builds. Users must uninstall the old debug app before installing it; Android removes the old app's data. Later release APKs must preserve the application ID, use this certificate (or a deliberately planned Android signing lineage), and keep an equal or higher `versionCode`. Increment `versionCode` for each new release, alongside the existing package-version updates. Do not replace the APK in an already published version to switch certificates. This setup does not change or publish any current release.

## Backup and recovery

The keystore and password are initially in the ignored `.release-private/` directory, with directory mode 0700 and file modes 0600 on Unix. They are local setup/recovery material, not an independent backup. Keep an encrypted off-device copy of the keystore and store its password separately in a password manager; verify recovery with `--check` before relying on that copy. Preserve the public certificate and digest too.

The key was created once with a random 384-bit password and a certificate lifetime of 10,000 days. `scripts/create-signing-key.py` refuses to overwrite any existing identity or pin. It is for first-time setup in a new project; do not rerun it to rotate this app's certificate. Losing or exposing the release key requires a deliberate recovery or signing-lineage plan. GitHub's UI/API do not return saved secret values, while authorized workflow code can access them, so keep the independent backup and review workflow changes carefully.

Environment ref restrictions do not prevent a maintainer from changing a trusted workflow or creating a matching tag. Protect/review release refs and workflow changes according to repository ownership. No required-reviewer gate or repository ruleset was added by this setup. Google Play distribution, if added later, should use Play App Signing and a separate upload key; this workflow currently distributes APKs through GitHub.

## References checked on 2026-11-07

- [Android: sign your app](https://developer.android.com/studio/publish/app-signing): protect and back up the app-signing key; distinguish app-signing and Play upload keys.
- [Android: how updates work](https://developer.android.com/google/play/app-updates): matching identity/certificate and version-code requirements.
- [Android: apksigner](https://developer.android.com/tools/apksigner): align before signing, verify signatures, and pass passwords with `env:`.
- [GitHub: secure use](https://docs.github.com/en/actions/reference/security/secure-use): least privilege, separate secrets, immutable dependencies, and untrusted workflow risks.
- [GitHub: managing environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments): scoped credentials and release-ref restrictions.
