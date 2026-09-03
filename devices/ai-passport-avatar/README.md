# AI Passport Agent Twin

This is an isolated ESP-IDF firmware target for the FoloToy AI Passport. It
turns the device into a physical mirror of Agent Bridge:

- the 240×320 display shows six Agent slots and a large color-coded title card;
- UP/DOWN chooses a live slot while skipping empty slots;
- `*` marks the slot selected in Codex while the bright border is the local target;
- holding/releasing OK sends the selected Agent press/release edges;
- BLE carries the existing fixed-size ABP v1 frames in ordered 20-byte GATT
  fragments;
- the center card shows the selected slot's UTF-8 session title, with two-line
  ellipsis for long names; its background color carries the Agent state;
- only the session title crosses the display side channel: no thread ID,
  prompt, response, terminal output, diff, or credential is sent to the device.

This directory is deliberately separate from `firmware/`, which remains the
ESP32-S3 USB H0 target. It shares only the repository's portable `abp`
component.

## Desktop pairing

BLE discovery is opt-in so normal Agent Bridge launches do not trigger a macOS
Bluetooth permission prompt:

```zsh
cd desktop-app
AGENT_BRIDGE_PASSPORT_BLE=1 npm run tauri dev
```

The app scans for `AgentPassport`, subscribes to its notification
characteristic, performs the normal ABP Hello/Snapshot handshake, and then
publishes a fresh VisualState every 750 ms and sends title packets only when a
slot title changes. The write characteristic requires an encrypted BLE link;
macOS may show a one-time pairing prompt.

For safe interaction testing without invoking real Codex controls, combine it
with the existing fake renderer:

```zsh
AGENT_BRIDGE_PASSPORT_BLE=1 AGENT_BRIDGE_H0_FAKE_RENDERER=1 npm run tauri dev
```

## Build and flash

Use ESP-IDF 5.5.3 (5.5.4 is also verified):

```zsh
cd devices/ai-passport-avatar
idf.py set-target esp32c3
idf.py build
```

Do not run `idf.py flash` against an unmodified production Passport: it also
writes this project's bootloader and partition table. The inspected 8 MB
factory unit had a 3 MB application at `0x10000`, followed by `imgstore`,
`imgframe`, `cardid`, `audio`, and `imgava` data partitions, and had no Recovery
image at `0x700000`. For first-device validation, make a full 8 MB backup and
write only `build/agent_passport_avatar.bin` at `0x10000`; leave the bootloader,
partition table, `cardid`, and factory resource partitions untouched. Never use
`erase-flash` on a production Passport.

The custom bootloader and Recovery partition in this target are for a later,
explicit full-install flow after a Recovery image has been prepared. They are
not part of the app-only first validation described above.

Only one physical Agent Bridge controller should be active during this MVP.
This is a personal-validation build: BLE uses Secure Connections bonding but
does not yet implement ABP H1 application-layer ownership/authentication.

## Verify portable logic

```zsh
devices/ai-passport-avatar/tests/native/run.sh
```

An ESP-IDF build additionally verifies the NimBLE, LVGL, ADC-button and ST7789
integration. Physical acceptance still requires the actual Passport hardware.

## BLE contract

Service: `6c6f7a10-5f6c-4d22-9bce-0a6ca7296f53`

- host → device: `6c6f7a11-5f6c-4d22-9bce-0a6ca7296f53`
- device → host: `6c6f7a12-5f6c-4d22-9bce-0a6ca7296f53`
- device info: `6c6f7a13-5f6c-4d22-9bce-0a6ca7296f53`

Each value has a five-byte header (`transfer`, `index`, `count`, `frame_len`,
`fragment_len`) followed by at most 15 bytes. A transfer is rejected on gaps,
reordering, inconsistent metadata, or more than 100 ms elapsed time.

ABP remains the frozen, content-free control protocol. This Passport target
also accepts a separate canonical 63-byte display packet on the same encrypted
write characteristic: magic `0xAD`, version `1`, kind `1` (Agent title), slot
`0..5`, UTF-8 byte length `0..57`, title bytes, then zero padding. The packet is
validated before display and never enters the ABP session sequence.
