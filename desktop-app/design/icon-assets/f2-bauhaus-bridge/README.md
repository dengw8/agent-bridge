# Agent Bridge F2 icon assets

Selected concept: **F2 — Balanced Bauhaus Bridge**.

## Contents

- `agent-bridge-f2-optical-83-master.png`: canonical 1254×1254 RGBA master; the borderless artwork is centered at 83% scale so Tauri development mode matches neighboring macOS Dock icons.
- `agent-bridge-f2-optical-94-master.png`: oversized optical-size trial retained as design history.
- `agent-bridge-f2-optical-90-master.png`: earlier, smaller optical-size trial retained as design history.
- `agent-bridge-f2-borderless-master.png`: full-bleed borderless source used to derive the optical master.
- `agent-bridge-f2-borderless-chroma.png`: magenta chroma-key intermediate retained for reproducibility.
- `agent-bridge-f2-master.png`: superseded first cutout; retained only as design history.
- `icon-manifest.json`: Tauri generation manifest; uses the icon blue instead of white for opaque mobile backgrounds.
- `tauri/`: Tauri-generated desktop, Windows, iOS, and Android assets.
  - Desktop: `32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png`, `icon.png`
  - macOS: `icon.icns`
  - Windows: `icon.ico`, `Square*Logo.png`, `StoreLogo.png`
  - Mobile: `ios/` and `android/`
- `macos/AgentBridge.iconset/`: the ten standard borderless macOS iconset source layers extracted from `icon.icns`.

The active files in `src-tauri/icons/` use the optical 83% F2 set.
