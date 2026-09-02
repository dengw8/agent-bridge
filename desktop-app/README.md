# Agent Bridge

Agent Bridge is the macOS personal-validation controller for the Codex UI in
ChatGPT Desktop. It is a Tauri 2 application with a React 19/Tailwind CSS 4
interface and a Rust backend. It does not require Stream Deck hardware and does
not use a Node sidecar.

## Run from source

Requirements: macOS 13 or newer, Node.js/npm, Rust, and ChatGPT Desktop.

```zsh
cd /Users/dengwang/github/agent-bridge/desktop-app
npm ci
npm run tauri dev
```

Agent Bridge never starts a closed ChatGPT application. If ChatGPT is already
running without its loopback debugging bridge, the app asks you to save unsent
Composer text and explicitly confirm the restart.

For a non-functional visual preview that never connects to ChatGPT:

```zsh
npm run dev -- --host 127.0.0.1
```

Then open `http://127.0.0.1:1420/?preview=1`.

## Build and open

```zsh
npm run tauri build
open "/Users/dengwang/github/agent-bridge/desktop-app/src-tauri/target/release/bundle/macos/Agent Bridge.app"
```

The generated app is an ad-hoc-signed local validation build, not a notarized
distribution. Its configuration and sanitized diagnostics are stored under
Agent Bridge's own macOS Application Support directory. Enabling “Open Agent
Bridge at login” is opt-in and creates only Agent Bridge's LaunchAgent. The
login preference can be changed later on the Configure page.

## Verify

```zsh
npm run check
npm test -- --run
npm run build
npm audit --omit=dev
cd src-tauri
cargo fmt --check
cargo check --locked
cargo test --locked
```

The environment-dependent read-only Renderer contract test is:

```zsh
cargo test --locked adapter::tests::live_renderer_snapshot_matches_the_typed_contract \
  -- --ignored --exact
```

It requires a running ChatGPT Desktop that already has loopback CDP enabled and
does not dispatch any UI action. On the validated build, Diagnostics shows 38
required native capabilities plus the optional Usage capability.

## Hardware development

The frozen USB/BLE scope, electronics, firmware, protocol, mechanical design,
delivery phases, and physical-validation responsibilities are defined in the
[hardware technical design](../docs/AGENT_BRIDGE_HARDWARE_TECHNICAL_DESIGN.md).
The older
[USB-only hardware/software interface draft](../docs/HARDWARE_SOFTWARE_INTERFACE.md)
is retained as superseded historical context. The H0 runtime opens only the
exact development VID/PID/Usage tuple through macOS IOHIDManager, keeps raw
handles and reports in Rust, and exposes only a content-free hardware status to
the WebView. The Diagnostics page distinguishes disconnected, permission
denied, protocol incompatible, unavailable, and ready states.

With the I02 firmware attached, the following helpers validate the frozen HID
descriptor and replay content-free VisualState values:

```zsh
cd src-tauri
cargo run --locked --bin h0-usb-evidence -- /path/to/new/evidence-directory
cargo run --locked --bin h0-visual-replay
```

## Security boundary

Renderer scripts are bundled resources selected by typed Rust commands.
Neither the WebView nor configuration exposes a raw CDP URL or arbitrary
JavaScript evaluation. CDP is accepted only from `127.0.0.1`, commands fail
closed when the Renderer disconnects, and diagnostics cannot accept task
titles, bodies, terminal output, diffs, audio, or credentials.
