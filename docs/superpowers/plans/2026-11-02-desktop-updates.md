# Desktop Updates Implementation Plan

**Goal:** Signed cross-platform updates, brand notification dot and native version title.
**Architecture:** A focused updater UI module uses the Tauri updater plugin to check
and download. Guarded native commands install and restart without losing ledger work.
**Tech Stack:** Tauri 2, official updater plugin, JavaScript, Rust, GitHub Actions.
**Spec:** ../specs/2026-11-02-desktop-updates-design.md

## Tasks

- [x] Write and run failing frontend tests for the dot, release notes, progress,
  confirmation, repeated clicks and retry. Implement `createUpdater` in `updater.js`
  with injected check/install/guard dependencies, then rerun tests.
- [x] Add native install preparation/restart commands and title from package version.
  Preserve existing busy/dirty exit behavior and run cargo tests.
- [x] Generate a persistent signing key outside this repository, configure its public
  key and enable updater artifacts. Store the key in the repository Actions secret.
- [x] Write failing release manifest tests for three platform payloads and missing
  signatures. Implement `scripts/release-manifest.js`; run the tests. Replace fixed
  release versions with metadata-driven draft publication followed by latest promotion.
- [x] Build a signed Windows installer, verify signatures and all tests; document
  first manual installation and maintainer release steps, then report remaining
  platform-specific verification limits. Do not publish without a user release request.

## Verification record

- 17 frontend/release tests and 11 Rust tests pass.
- Vite and signed Windows NSIS builds pass; independent Ed25519 verification accepts
  the final package and rejects a modified copy.
- actionlint passes for the complete workflow.
- Native debug title observed as “运管家 v0.1.2”, using a disposable QA ledger path.
- Independent read-only review found an early macOS exit permission issue; fixed by
  authorizing exit only in the final restart command, with regression coverage.
- macOS runtime and real published old-to-new upgrade verification remain outstanding.
- No commit, push, tag, or public release was performed for this feature.
