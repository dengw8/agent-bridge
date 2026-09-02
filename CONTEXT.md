# Agent Bridge

Agent Bridge is a local macOS control system that connects virtual or physical controls to the user's current ChatGPT Desktop experience and returns bounded status feedback. This glossary defines the product language, not the current implementation state.

## Product boundary

**Agent Bridge System**:
The complete product formed by the Agent Bridge App and an optional Agent Bridge Device.
_Avoid_: Agent Bridge, when the App/device distinction matters; Codex keyboard

**Agent Bridge App**:
The local macOS control application that owns configuration, resolves input meaning, controls ChatGPT Desktop, and manages Agent Bridge Devices.
_Avoid_: Agent Bridge, desktop bridge, host UI

**Agent Bridge Device**:
The physical self-use controller that emits control-position semantics and renders status supplied by the Agent Bridge App.
_Avoid_: keyboard, macro pad, Codex Micro clone

**ChatGPT Desktop**:
The installed ChatGPT macOS application whose existing Codex interface is controlled by the Agent Bridge App.
_Avoid_: Codex Desktop, Agent host, Renderer

**ChatGPT Bridge**:
The Agent Bridge App's current, local control relationship with a compatible ChatGPT Desktop instance.
_Avoid_: Bridge, connection, device bridge

**Capability**:
A ChatGPT Desktop behavior that the Agent Bridge App has positively confirmed is available in the current application generation.
_Avoid_: feature flag, assumed support

## Controls and configuration

**Agent Slot**:
One of six host-defined positions that may refer to a current ChatGPT task or be empty.
_Avoid_: Native Slot, task slot, agent

**Agent Key**:
A physical or virtual control associated with exactly one Agent Slot, identified as AG00 through AG05.
_Avoid_: task key, thread key

**Action Position**:
One of seven fixed physical control positions, identified independently as ACT06 through ACT12.
_Avoid_: Command Slot, macro key

**Command Key**:
A user-facing logical control whose meaning is resolved by the Agent Bridge App from an Action Position or the combined Mic Pair.
_Avoid_: Action Position, keyboard shortcut, macro

**Mic Pair**:
The adjacent ACT10 and ACT11 Action Positions that can act as one dual-wide Mic control or two independent Command Keys.
_Avoid_: 2U key, ACT10_ACT11

**Combined Mic Mode**:
The Mic Pair mode in which either physical switch begins one logical Mic hold and the logical hold ends only after both switches are released.
_Avoid_: merged switch, firmware Mic

**Split Mic Mode**:
The Mic Pair mode in which ACT10 and ACT11 are independent Command Keys.
_Avoid_: dual Mic, separate layer

**Profile Key**:
The dedicated physical control used to select a Connection Mode, manage the selected BLE Slot, and confirm physical presence during Claim.
_Avoid_: Profile, Bluetooth key, mode key

**Physical Keycap**:
A removable cap installed on a physical switch, including the custom dual-stem Mic cap.
_Avoid_: Keycap Action, Binding

**Keycap Action**:
One of the named software actions offered by the ChatGPT/Codex control vocabulary and selectable as a Binding.
_Avoid_: Physical Keycap, key code, shortcut

**Binding**:
An App-owned association from a configurable logical control to a Keycap Action, command, skill, or no action.
_Avoid_: macro, firmware mapping, shortcut

**Layout**:
The Agent Bridge App's single active collection of Command Key, joystick, and encoder configuration.
_Avoid_: Profile, layer, keymap

**Encoder Mode**:
The App-owned interpretation selected for encoder rotation and press behavior.
_Avoid_: encoder firmware mode, hardware layer

**Voice Button Mode**:
The App-owned choice between push-to-talk and voice-chat behavior for the logical Mic control.
_Avoid_: Mic Pair mode, recording state

## Input and session language

**Physical Event**:
A function-independent control occurrence such as a position being pressed, released, entered, centered, or rotated.
_Avoid_: command, shortcut, Intent

**Input Source**:
A producer of Physical Events, such as the virtual UI or one active device connection.
_Avoid_: transport, device, producer

**Connection Generation**:
One uninterrupted lifetime of an Input Source connection; a reconnect or device restart creates a new generation.
_Avoid_: session ID, sequence, boot

**Transport**:
The bidirectional carrier used by an Agent Bridge Device and App to exchange protocol messages.
_Avoid_: Connection Mode, BLE Slot, Device Session

**Connection Mode**:
The explicit device selection among USB, BT1, BT2, and BT3; attaching USB power does not itself change this selection.
_Avoid_: Profile, transport, active host

**BLE Slot**:
One of the three device-held Bluetooth host relationships selected by BT1, BT2, or BT3.
_Avoid_: Profile, Connection Mode, device slot

**Active Device**:
The single trusted Agent Bridge Device currently permitted to contribute physical input to the Agent Bridge App.
_Avoid_: connected device, selected host

**Device Session**:
The logical relationship between the Agent Bridge App and one device generation from transport connection through compatibility, authentication, reconciliation, and disconnection.
_Avoid_: Transport, BLE Bond, Connection Generation

**Ready Session**:
A Device Session that is compatible, authenticated, reconciled with current held inputs, and permitted to produce normal input.
_Avoid_: connected, paired, bonded

**Input Snapshot**:
A complete statement of the device's currently held or entered controls at one point in a Device Session.
_Avoid_: event history, replay, App snapshot

**Intent**:
The semantic action the Agent Bridge App resolves from a Physical Event, the active Layout, and current ChatGPT Desktop context.
_Avoid_: Physical Event, Binding, shortcut

## Trust and safety

**Unclaimed Device**:
An Agent Bridge Device that has no established Host Trust and therefore cannot produce normal input.
_Avoid_: unpaired, disconnected, unauthorized session

**Claim**:
The first establishment of Host Trust initiated in the Agent Bridge App and completed only after physical confirmation on the target device.
_Avoid_: BLE pairing, connect, login

**Physical Confirmation**:
An intentional Profile Key action proving that the user is physically present at the device for a pending Claim or destructive slot operation.
_Avoid_: consent dialog, button click, pairing

**Host Trust**:
The persistent application-level trust relationship between one Agent Bridge Device and one macOS host.
_Avoid_: BLE Bond, login, device permission

**BLE Bond**:
The operating-system Bluetooth relationship that protects a BLE link; it is necessary for BLE but does not replace Host Trust.
_Avoid_: Claim, Host Trust, paired device

**Offline Device**:
A previously usable Agent Bridge Device whose App relationship is not currently healthy enough to accept input or display live executable state.
_Avoid_: disconnected device, stale-but-ready

**Incompatible Device**:
An Agent Bridge Device whose protocol or hardware relationship cannot safely enter a Ready Session with the current App.
_Avoid_: offline, unauthenticated, failed device

## Feedback and status

**Visual State**:
The complete, content-free semantic feedback supplied by the Agent Bridge App for device-local rendering.
_Avoid_: LED command, animation frame, task payload

**Agent Status**:
The bounded visual condition of an Agent Slot, such as empty, idle, thinking, complete, requires input, or error.
_Avoid_: task content, native status string

**Mic Status**:
The bounded visual condition of the current ChatGPT Desktop Mic interaction, such as idle, recording, processing, ready, locked, stopping, or unknown.
_Avoid_: Voice Button Mode, recording content

**Device Status**:
The bounded operational condition reported by an Agent Bridge Device, including connection health, power, battery, charging, version, calibration, and reset information.
_Avoid_: Visual State, diagnostics dump, raw report

## Prototypes, manufacturing, and acceptance

**H0 Prototype**:
The development-board electrical prototype used to prove the protocol, real USB path, representative controls, and disconnect safety.
_Avoid_: breadboard demo, Rev A

**H1 Prototype**:
The full-function modular bench prototype used to prove all controls, BLE slots, trust, lighting, battery behavior, settings, update/recovery, and full-size mechanical fit.
_Avoid_: Rev A, final prototype

**Rev A**:
The first integrated custom-PCB and enclosure revision used for bring-up and design verification.
_Avoid_: prototype board, production version

**Rev B**:
The corrected daily-use hardware revision intended for the final 1–5 self-use devices.
_Avoid_: production product, certified device, mass-production revision

**Gate**:
An evidence-backed phase decision that prevents higher-cost work from starting until all blocking acceptance criteria are closed.
_Avoid_: milestone, demo, informal approval

**Physical Gate**:
A Gate whose result requires real components, assembly, measurement, human judgment, multiple Macs, or an external manufacturing service.
_Avoid_: manual test, optional validation

**Factory Self-Test**:
A repeatable device-level acceptance check that verifies measurable hardware functions without requiring real ChatGPT side effects.
_Avoid_: system acceptance, smoke test, diagnostics

**Production Package**:
The versioned, reproducible collection of source and generated artifacts required to manufacture, assemble, provision, recover, and test a hardware revision.
_Avoid_: Gerbers, release zip, design files

**Recovery**:
The approved process for returning a device with unusable normal firmware to a known signed and testable state.
_Avoid_: update, reset, downgrade
