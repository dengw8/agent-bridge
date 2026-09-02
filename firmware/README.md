# Agent Bridge H0 firmware

This ESP-IDF project targets `ESP32-S3-DevKitC-1-N8R8`. It exposes one
development-only Vendor HID collection and no keyboard, mouse, CDC, or mass
storage business interface.

Requirements: ESP-IDF 5.5.x, Node.js, and a native USB data connection on
GPIO19 (D-) / GPIO20 (D+).

```sh
node descriptors/generate.mjs --check
idf.py set-target esp32s3
idf.py build
idf.py -p /dev/cu.YOUR_PORT flash monitor
```

The BOOT button on GPIO0 is the temporary AG00 input. Holding it while the
board resets enters the ROM download mode; release it before a normal reset.
The WS2812B data output is GPIO18 and must pass through the enabled 3.3 V to
5 V `74AHCT125N` channel described in the I02 physical acceptance checklist.
Firmware clamps effective LED brightness to 25 percent.

Follow [I02_PHYSICAL_ACCEPTANCE.md](I02_PHYSICAL_ACCEPTANCE.md) before applying
power to the level shifter or LED board.

Run the host-buildable session and visual tests without ESP-IDF:

```sh
tests/h0_native/run.sh
```
