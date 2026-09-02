# I02 physical build and acceptance

This checklist is the human-executed part of I02. Keep the H0 app in Fake
Renderer mode until USB, input, and lighting behavior is proven; this prevents
the temporary button from causing a real Codex action.

## Wiring (all power disconnected)

Use GPIO18 only for the WS2812B data path. On a DIP-14 `74AHCT125N`:

| Pin | Connection |
|---:|---|
| 14 (`VCC`) | DevKit/USB `5V` |
| 7 (`GND`) | DevKit `GND` and WS2812B `GND` |
| 1 (`/1OE`) | `GND` (channel 1 enabled) |
| 2 (`1A`) | ESP32-S3 GPIO18 |
| 3 (`1Y`) | 220–470 ohm series resistor, then WS2812B `DIN`/`IN` |
| 4, 10, 13 (unused `/OE`) | `5V` (unused channels disabled) |
| 5, 9, 12 (unused inputs) | `GND` |
| 6, 8, 11 (unused outputs) | Not connected |

Place a 100 nF ceramic capacitor between pins 14 and 7 close to the IC. Connect
WS2812B `5V` to the same USB 5 V rail and verify that the data wire enters the
board at `DIN`/`IN`, not `DOUT`/`OUT`. The firmware uses the DevKit BOOT button
on GPIO0 as temporary AG00.

### Damage-prevention checks

- Do not connect an external 5 V supply while USB 5 V is connected unless the
  supplies are explicitly isolated. I02 assumes one USB-powered 5 V source.
- Never place 5 V on an ESP32-S3 GPIO. Pin 3 of the `74AHCT125N` goes toward
  the LEDs; pin 2 goes toward GPIO18.
- Confirm the DIP notch/dot and pin numbers before applying power. Reversed IC
  orientation can short 5 V to ground or a GPIO.
- Check continuity for common ground and check that 5 V is not shorted to GND
  before connecting USB. Assemble and change wiring only while unpowered.
- Do not bypass the series resistor or leave unused AHCT inputs/enables
  floating. Keep the first test at the firmware-enforced 25% brightness cap.
- GPIO0 is a boot strap. Holding BOOT while resetting enters download mode; it
  does not represent a firmware failure. Release BOOT and reset for normal boot.
- Use the native USB path (GPIO19 D-, GPIO20 D+) for ABP. Do not configure the
  same native PHY as USB Serial/JTAG at the same time.

## Build, flash, and smoke test

From `firmware/` in an ESP-IDF 5.5.x environment:

```sh
node descriptors/generate.mjs --check
idf.py set-target esp32s3
idf.py fullclean
idf.py build
idf.py -p /dev/cu.YOUR_FLASH_PORT flash monitor
```

Pass criteria: the build is clean, boot logs contain the development Vendor HID
line, the board does not enter a reset loop, and connecting the native USB port
logs `USB mounted; awaiting ABP snapshot handshake` without exposing a keyboard,
mouse, CDC, or mass-storage business interface.

## macOS and end-to-end test

1. Start the app without real Codex side effects:

   ```sh
   cd ../desktop-app
   AGENT_BRIDGE_H0_FAKE_RENDERER=1 npm run tauri dev
   ```

2. Open Diagnostics. Confirm the hardware state progresses from `disconnected`
   to `connecting` and then `available`. If macOS refuses the open, it must show
   `permissionDenied`; a mismatched ABP major/minor must show `incompatible`.
3. Capture frozen descriptor evidence into a new empty directory:

   ```sh
   cd src-tauri
   cargo run --locked --bin h0-usb-evidence -- /path/to/new/i02-evidence
   /usr/sbin/ioreg -p IOUSB -l -w0 \
     -n "Agent Bridge H0 Vendor HID Dev" \
     > /path/to/new/i02-evidence/ioreg-h0-usb.txt
   ```

   Before sharing the IORegistry file, verify that no unrelated device section,
   serial number, or personal device name is present. The H0 section must show
   development VID `0xcafe` and PID `0xab01`.
4. Press and release BOOT once. Pass criteria: one AG00 Pressed and one Released
   are counted, the Fake Renderer produces a receipt, and unplugging never leaves
   AG00 held.
5. Run `cargo run --locked --bin h0-visual-replay` and observe all states. Pass
   criteria: connection is visible, AG00/Agent activity is visible, Error is red,
   VisualState returns to the device, and no content string is displayed.
6. Repeat enumeration and bidirectional communication with two known data
   cables, once directly connected to the Mac and once through the intended hub.
   Record cable/path, descriptor result, AG00 edge result, visual result, and any
   reset/brownout.

I02 physical acceptance requires all four cable/path trials to remain enumerated
through the full replay, with no USB dropout, ESP32 reset, or brownout message.
