# Agent Bridge Protocol v1

Status: Frozen v1.0
Wire version: `1.0`
Frame size: 63 bytes
Authority: this document is the byte-level expansion of
`AGENT_BRIDGE_HARDWARE_TECHNICAL_DESIGN.md` section 10.

## 1. Scope and invariants

ABP carries physical control semantics and content-free device state between one
Agent Bridge device and the macOS App. USB Vendor HID and BLE GATT carry the
same ABP frames. Neither transport changes message semantics.

ABP never carries a ChatGPT title, thread ID, account, Binding, command name,
Keycap ID, prompt, response, terminal content, diff, credential, or system
shortcut. Firmware reports physical positions only. The App owns all mapping to
existing `ControlEngine` intents.

All receivers:

- require an exact, bounded frame or fragment length;
- reject unknown values rather than guessing;
- validate the complete frame before changing session or input state;
- ignore duplicate and out-of-order frames;
- enter snapshot recovery on a sequence gap or device reboot;
- clear only the disconnected/desynchronized source's held contributions;
- never replay input after reconnect, reboot, or snapshot recovery.

## 2. Integer and canonical encoding rules

- All multi-byte integers are unsigned little-endian unless explicitly marked
  signed.
- `i16` uses two's-complement little-endian.
- Booleans are exactly `0` or `1`.
- Reserved bytes and unused bits MUST be zero.
- Every enum value not listed here is unknown and MUST be rejected.
- Payload padding from `14 + payload_length` through byte 61 MUST be zero.
- Encoders MUST emit the unique canonical representation described here.

## 3. Common frame

An ABP application frame is always exactly 63 bytes.

| Offset | Size | Field | Rule |
|---:|---:|---|---|
| 0 | 1 | magic | `0xAB` |
| 1 | 1 | protocol_major | `0x01` |
| 2 | 1 | protocol_minor | `0x00` |
| 3 | 1 | message_type | section 5 |
| 4 | 1 | flags | `0x00` in v1.0 |
| 5 | 1 | payload_length | `0..48` |
| 6 | 4 | sequence | `1..0xffffffff` |
| 10 | 4 | sender_uptime_ms | wraps naturally |
| 14 | 48 | payload and zero padding | message-specific |
| 62 | 1 | reserved | `0x00` |

`sequence=0`, a non-zero flag/reserved/padding byte, a frame shorter or longer
than 63 bytes, or `payload_length>48` is invalid.

### 3.1 Version compatibility

- A receiver supports frames with its exact major and a minor less than or
  equal to its implemented minor.
- A major mismatch enters `Incompatible`; it is never downgraded.
- A higher minor is rejected until a compatibility review explicitly adds it.
- v1.0 therefore accepts only `1.0`.
- A breaking field, enum, state-machine, or security change requires a new
  major. A backward-compatible addition requires a new minor and must preserve
  v1.0 canonical packets.

## 4. Transport wrappers

### 4.1 USB Vendor HID

The USB report is exactly 64 bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | Report ID |
| 1 | 63 | ABP frame |

- Device → Host Input Report ID: `0x01`.
- Host → Device Output Report ID: `0x02`.
- Wrong Report ID or length is rejected before ABP decoding.

### 4.2 BLE GATT fragmentation

If one characteristic value cannot carry 63 bytes, `BleGattAdapter` prepends
this five-byte header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | transfer_id |
| 1 | 1 | fragment_index, starting at 0 |
| 2 | 1 | fragment_count, `1..16` |
| 3 | 1 | frame_length, exactly 63 |
| 4 | 1 | fragment_length |
| 5 | N | fragment bytes |

The adapter fixes the chunk capacity when a transfer starts. Non-final
fragments have that full capacity; the final fragment has the exact remainder.
Fragments must arrive in increasing index order with matching transfer ID,
count, and frame length. Duplicate, reordered, missing, inconsistent, oversized,
or more-than-100-ms-old transfers discard the complete reassembly buffer. The
buffer is fixed at 63 bytes and no partial frame reaches ABP decoding.

## 5. Message types and direction

| Value | Direction | Message |
|---:|---|---|
| `0x01` | Device → Host | `DEVICE_HELLO` |
| `0x02` | Device → Host | `AUTH_RESPONSE` |
| `0x03` | Device → Host | `INPUT_EVENT` |
| `0x04` | Device → Host | `INPUT_SNAPSHOT` |
| `0x05` | Device → Host | `DEVICE_STATUS` |
| `0x06` | Device → Host | `PONG` |
| `0x07` | Device → Host | `ACK` |
| `0x08` | Device → Host | `PROTOCOL_ERROR` |
| `0x09` | Device → Host | `UPDATE_STATUS` |
| `0x40` | Host → Device | `HOST_HELLO` |
| `0x41` | Host → Device | `AUTH_CHALLENGE` |
| `0x42` | Host → Device | `CLAIM_BEGIN` |
| `0x43` | Host → Device | `CLAIM_COMMIT` |
| `0x44` | Host → Device | `GET_INPUT_SNAPSHOT` |
| `0x45` | Host → Device | `GET_DEVICE_STATUS` |
| `0x46` | Host → Device | `VISUAL_STATE` |
| `0x47` | Host → Device | `SET_DEVICE_CONFIG` |
| `0x48` | Host → Device | `PING` |
| `0x49` | Host → Device | `HOST_GOING_AWAY` |
| `0x4a` | Host → Device | `UPDATE_BEGIN` |
| `0x4b` | Host → Device | `UPDATE_CHUNK` |
| `0x4c` | Host → Device | `UPDATE_COMMIT` |

A transport endpoint rejects a known message sent in the wrong direction.
Every unknown type is invalid.

## 6. Control and input enums

### 6.1 Control kind and ID

| Kind | Value | Valid IDs |
|---|---:|---|
| Agent | `0` | `0..5` = `AG00..AG05` |
| Action | `1` | `0..6` = `ACT06..ACT12` |
| Joystick | `2` | `0=Up`, `1=Right`, `2=Down`, `3=Left` |
| EncoderPress | `3` | `0` |
| EncoderRotate | `4` | `0` |

`ACT10` and `ACT11` are separate wire controls. Their optional logical merge is
host-only.

### 6.2 Phase

| Phase | Value | Valid control |
|---|---:|---|
| Pressed | `0` | Agent, Action, EncoderPress |
| Released | `1` | Agent, Action, EncoderPress |
| Entered | `2` | Joystick |
| Centered | `3` | Joystick |
| Rotated | `4` | EncoderRotate |

The value field is zero for all edges. `Rotated` requires a non-zero `i16`
between -32 and +32 inclusive.

## 7. Payload schemas

All offsets below are relative to payload byte zero. A listed exact length is
mandatory.

### 7.1 Device → Host

#### `DEVICE_HELLO` — 24 bytes

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `boot_id`, non-zero |
| 4 | 16 | stable device ID (backend-only) |
| 20 | 2 | hardware revision |
| 22 | 1 | firmware major |
| 23 | 1 | firmware minor |

An all-zero device ID is invalid.

#### `AUTH_RESPONSE` — 36 bytes

`boot_id:u32` followed by a 32-byte HMAC-SHA256 response.

#### `INPUT_EVENT` — 9 bytes

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `boot_id`, non-zero |
| 4 | 1 | control kind |
| 5 | 1 | control ID |
| 6 | 1 | phase |
| 7 | 2 | value `i16` |

#### `INPUT_SNAPSHOT` — 8 bytes

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `boot_id`, non-zero |
| 4 | 1 | Agent pressed mask, bits 0..5 |
| 5 | 1 | Action pressed mask, bits 0..6 |
| 6 | 1 | Joystick entered mask, bits 0..3; at most one bit |
| 7 | 1 | EncoderPress, `0` or `1` |

Snapshot-held controls are suppressed until release when a connection first
becomes Ready. A recovery snapshot re-establishes producer truth without
synthesizing missed presses.

#### `DEVICE_STATUS` — 10 bytes

`boot_id:u32`, connection mode `u8`, battery percent `u8`, power flags `u8`,
reset reason `u8`, bounded error count `u16`. Battery is `0..100` or `255`
(unknown). Power flags use bits 0=charging, 1=external power, 2=low battery.

Connection mode is `0=USB`, `1=BT1`, `2=BT2`, `3=BT3`.

#### `PONG` — 8 bytes

`boot_id:u32`, then the exact `PING` token `u32`.

#### `ACK` — 9 bytes

`boot_id:u32`, acknowledged sequence `u32`, status `u8` where
`0=accepted`, `1=rejected`, `2=busy`.

#### `PROTOCOL_ERROR` — 10 bytes

`boot_id:u32`, offending sequence `u32`, error code `u8`, bounded detail `u8`.
Error codes are `1=malformed`, `2=unsupportedVersion`, `3=unknownMessage`,
`4=invalidValue`, `5=sequence`, `6=notReady`, `7=unauthenticated`,
`8=busy`, and `9=internal`.

#### `UPDATE_STATUS` — 8 bytes

`boot_id:u32`, update state `u8`, progress percent `u8`, error code `u8`,
reserved `u8=0`. State is `0=idle`, `1=receiving`, `2=verifying`,
`3=readyToReboot`, `4=failed`, `5=rolledBack`.

### 7.2 Host → Device

#### `HOST_HELLO` — 20 bytes

Host instance nonce `[u8;16]` followed by feature bits `u32`. The nonce cannot be
all zero. v1.0 defines feature bit 0=`fullVisualState`; all other bits are zero.

#### `AUTH_CHALLENGE` — 32 bytes

A non-zero 32-byte random challenge.

#### `CLAIM_BEGIN` — 48 bytes

A 15-byte random nonce followed by a 33-byte SEC1 compressed P-256 ephemeral
public key. Prefix must be `0x02` or `0x03`; nonce and X coordinate cannot be
all zero. The 120-bit nonce is unique within the 30-second physical claim
window.

#### `CLAIM_COMMIT` — 32 bytes

The non-zero HMAC-SHA256 transcript confirmation.

#### `GET_INPUT_SNAPSHOT` — 0 bytes

Any payload is invalid.

#### `GET_DEVICE_STATUS` — 0 bytes

Any payload is invalid.

#### `VISUAL_STATE` — 20 bytes

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | visual revision |
| 4 | 6 | AG00..AG05 visual status |
| 10 | 1 | selected Agent mask, bits 0..5 |
| 11 | 1 | Mic state |
| 12 | 1 | Bridge state |
| 13 | 1 | connection state |
| 14 | 1 | active profile |
| 15 | 1 | battery percent |
| 16 | 1 | power flags |
| 17 | 1 | brightness percent |
| 18 | 2 | auto-dim seconds, `0..3600` |

Agent status: `0=Empty`, `1=Idle`, `2=Thinking`, `3=Complete`,
`4=RequiresInput`, `5=Error`.

Mic state: `0=Idle`, `1=Recording`, `2=Processing`, `3=Ready`, `4=Locked`,
`5=Stopping`, `6=Unknown`.

Bridge state: `0=Offline`, `1=Ready`, `2=Degraded`, `3=Incompatible`.

Connection state: `0=Offline`, `1=Ready`, `2=Pairing`, `3=Claiming`,
`4=Updating`.

Profile: `0=USB`, `1=BT1`, `2=BT2`, `3=BT3`.

Battery and power flags follow `DEVICE_STATUS`. Brightness is `0..100`.
This is the complete v1 visual payload. Its closed numeric schema makes
ChatGPT/user content structurally unrepresentable.

#### `SET_DEVICE_CONFIG` — 6 bytes

Brightness percent `u8`, auto-dim seconds `u16`, profile `u8`, input mode
`u8`, reserved `u8=0`. Input mode is `0=normal`, `1=calibration`.

#### `PING` — 4 bytes

An opaque token `u32`.

#### `HOST_GOING_AWAY` — 1 byte

Reason: `0=quit`, `1=sleep`, `2=transportSwitch`, `3=update`.

#### `UPDATE_BEGIN` — 44 bytes

Image size `u32` (non-zero), hardware revision `u16`, semantic version
major/minor/patch `u8` each, image flags `u8` (bit 0=development), image
SHA-256 `[u8;32]`, chunk size `u16` in `1..43`.

#### `UPDATE_CHUNK` — 5..48 bytes

Offset `u32`, data length `u8` in `1..43`, then exactly that many data bytes.

#### `UPDATE_COMMIT` — 32 bytes

The exact non-zero SHA-256 digest declared by `UPDATE_BEGIN`. The signed image
and signature bytes are transported as update chunks and verified before
commit; no update state changes on a malformed commit.

## 8. Sequence, boot, snapshot, and generation

Each direction has an independent `u32` sequence. A new connection generation
starts at 1. Senders must reconnect before sequence exhaustion; wrap to zero is
forbidden.

For a given Rust-assigned source and generation:

1. The receiver accepts the next exact sequence only.
2. A duplicate (`==last`) or out-of-order (`<last`) frame is discarded without
   state change.
3. A gap (`>last+1`) discards that frame, clears that source's held
   contributions, enters `AwaitingSnapshot`, and requests a snapshot.
4. No `INPUT_EVENT` is executable until a valid snapshot restores Ready.

`boot_id` is non-zero and chosen freshly on every firmware boot. A changed
`boot_id` clears that source's contributions, invalidates its prior sequence
and snapshot truth, and requires a new `DEVICE_HELLO` plus `INPUT_SNAPSHOT`.
Frames from an older boot remain invalid.

Connection generation is a host-only `u64` assigned by the concrete Rust
USB/BLE/Fake adapter. It is not accepted from ABP or Tauri IPC. A stale
generation cannot address a current session.

## 9. Session states

The v1 states are `Disconnected`, `AwaitingHello`, `AwaitingSnapshot`, `Ready`,
`Offline`, `Incompatible`, `Claiming`, and `Updating`.

Minimum Fake/H0 establishment:

```text
Connected(generation assigned by Rust)
  → DEVICE_HELLO
  → GET_INPUT_SNAPSHOT
  → INPUT_SNAPSHOT
  → Ready
```

Claim/auth messages are frozen here but their cryptographic state machine is
implemented by I-09. Update messages are frozen here but update storage,
signature verification, rollback, and Recovery are implemented by I-15.

## 10. Golden Packets and invalid corpus

`protocol/golden/manifest.json` is the readable source for canonical packets.
`protocol/golden/*.bin` are generated exact 63-byte frames. Rust and C tests
decode each binary, compare its manifest fields, re-encode it, and require
byte-for-byte equality.

The invalid corpus and property tests cover:

- all truncation lengths 0..62 and an oversized 64-byte frame;
- wrong magic, version, type, flags, payload length, padding, or reserved byte;
- every invalid control/phase/value combination and out-of-range enum;
- zero `boot_id`, zero required nonce/digest, invalid masks, and invalid
  variable-length update chunks;
- duplicate, out-of-order, gap, sequence exhaustion, reboot, stale generation,
  and disconnect while held;
- random 63-byte inputs with no panic, out-of-bounds access, undefined behavior,
  or state mutation on rejection.

Every message in section 5 has a legal Golden Packet. Its invalid test is the
same packet with at least one required length, enum, range, zero/non-zero, or
direction invariant violated.
