# Desktop updates and releases

## User behavior

The native title displays the installed version. After startup and every six hours,
the updater checks the latest stable GitHub release's `latest.json`. A higher version
adds a red dot to the brand icon. Click it to read notes, then choose “下载并更新”.
Settings also provides a manual check. Background network failures never interrupt
ledger work. Downloads are verified with the bundled updater public key before
installation. Installation requires confirmation if a form has unsaved changes and
will wait for ledger operations to finish. Windows uses the passive installer;
macOS replaces the application bundle and restarts.

Version 0.1.1 has no updater. Install 0.1.2 manually once to join this update channel.
Updating does not move the ledger or change the application identifier.

## Maintainer setup

- The updater public key is in `src-tauri/tauri.conf.json`.
- The existing private key is backed up outside the repository at
  `%USERPROFILE%/.codex/signing-keys/car-updater.key`, with a matching `.pub` file.
  Keep a secure backup: replacing this key will break updates for installed clients.
- GitHub Actions uses repository secret `TAURI_SIGNING_PRIVATE_KEY`; this key has no
  password. The workflow also accepts `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if a
  password-protected key is configured in a coordinated future key migration.
- This signature is for updater integrity. It is separate from Windows Authenticode
  or Apple Developer signing and notarization, which remain unconfigured.

For a locally signed Windows bundle:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $env:USERPROFILE '.codex/signing-keys/car-updater.key'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
npm run desktop:build -- --bundles nsis
```

For test bundles without access to the private key, disable update artifact creation
for that build only:

```powershell
npm run desktop:build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

## Publish a version

1. Increment the version consistently in package.json, package-lock.json,
   src-tauri/tauri.conf.json, Cargo.toml and Cargo.lock. Use a new stable version for
   every published release. Update RELEASE_NOTES.md and run npm/cargo tests.
2. Push the reviewed code to main. Push a matching `vX.Y.Z` tag, or run Desktop builds
   manually on the intended commit with `publish` enabled. Ordinary pushes and PRs
   build test installers without exposing the signing key.
3. The three builds upload the Windows installer, its signature, both Mac DMGs and
   signed `.app.tar.gz` bundles. The release job validates version agreement, tag,
   platform completeness and signatures, and creates platform-specific URLs.
4. A draft release receives all assets and `latest.json` before becoming public and
   latest. Existing releases are never overwritten or deleted. If an upload fails,
   the draft is kept for inspection; remove the failed draft before rerunning that
   same unpublished version. Roll back a bad public release by publishing a higher
   fixed version, never by silently replacing an already published payload.

## Validation

Run `npm test`, `npm run build` and `cargo test --locked --manifest-path
src-tauri/Cargo.toml`. Release tests exercise CI artifact collection and block an
incomplete update manifest. Updater UI tests cover notes as plain text, silent
failures, progress, repeated clicks, retry, and cancelling a dirty-work confirmation.
Native tests verify an update cannot reserve installation while ledger work is busy
and that installation reservations block new ledger work until released.

Before a production rollout, exercise a signed older-to-newer update on Windows and
each Mac architecture. Verify restart, ledger contents, no-update, offline, cancelled
confirmation and install failure. A local Windows build alone does not validate macOS
replacement, Gatekeeper behavior, or a complete public release-to-release upgrade.
