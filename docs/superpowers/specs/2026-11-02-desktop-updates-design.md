# 运管家桌面更新设计

Approved in chat: Windows and macOS; show release notes before the user clicks
“下载并更新”. Include a version in the native window title, read from package metadata.

- Check the signed GitHub latest.json after boot and every six hours; failures stay
  silent unless the user opens the update dialog. Only higher stable versions count.
- Show a red dot above the brand icon. Clicking it opens notes and an explicit update
  action; the original homepage link works when no update is available.
- Download with progress, verify the official updater signature, then install and
  restart. Protect unsaved forms and in-flight ledger operations before installation.
  Failed downloads or installs remain retryable. Render remote notes as plain text.
- Use Tauri updater on Windows x64, macOS arm64 and macOS x64. Signed NSIS and
  .app.tar.gz payloads plus latest.json are published by versioned release automation.
  Keep historical releases and do not mutate existing versioned releases.
- First updater-enabled version is 0.1.2, including the pending date calendar change.
  Existing 0.1.1 clients need one manual install. Signing private key lives outside
  the checkout and in GitHub Actions secrets; only its public key is bundled.
- Validate frontend lifecycle/error/dirty/busy cases, release manifest generation,
  Rust ledger tests, and signed Windows packaging. macOS execution requires a Mac.
