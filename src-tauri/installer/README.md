# Windows installer branding

`sidebar.bmp` (164 × 314) and `header.bmp` (150 × 57) are opaque 24-bit
BMPs consumed by Tauri's standard NSIS template. The sidebar is shared by
the welcome and finish pages. The header is used on intermediate pages.
The installer uses `../icons/icon.ico` through `tauri.conf.json`.

To regenerate from the existing app icon, install Pillow 12.3.0 and run:

```powershell
python scripts/generate-installer-assets.py
```

The script resolves paths relative to its own location. It defaults to Windows'
Microsoft YaHei Bold (`C:/Windows/Fonts/msyhbd.ttc`); `--font` accepts an explicit
path to that font on another machine. Commit the generated BMPs when changing
branding. Normal local and CI builds do not need the generator or its dependencies.

Build with `npm run desktop:build -- --bundles nsis`. For release QA, inspect
welcome, directory, progress, and finish pages at Windows scaling of 100%, 150%,
and 200%; test clean installation, upgrade, launch, and uninstall in a disposable
Windows VM. Do not use an existing personal ledger for installation tests.
