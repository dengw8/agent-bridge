use std::fmt;

pub const FRAME_SIZE: usize = 63;
pub const PAYLOAD_CAPACITY: usize = 48;
pub const USB_REPORT_SIZE: usize = 64;
pub const USB_INPUT_REPORT_ID: u8 = 0x01;
pub const USB_OUTPUT_REPORT_ID: u8 = 0x02;
pub const PROTOCOL_MAJOR: u8 = 1;
pub const PROTOCOL_MINOR: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    DeviceHello = 0x01,
    AuthResponse = 0x02,
    InputEvent = 0x03,
    InputSnapshot = 0x04,
    DeviceStatus = 0x05,
    Pong = 0x06,
    Ack = 0x07,
    ProtocolError = 0x08,
    UpdateStatus = 0x09,
    HostHello = 0x40,
    AuthChallenge = 0x41,
    ClaimBegin = 0x42,
    ClaimCommit = 0x43,
    GetInputSnapshot = 0x44,
    GetDeviceStatus = 0x45,
    VisualState = 0x46,
    SetDeviceConfig = 0x47,
    Ping = 0x48,
    HostGoingAway = 0x49,
    UpdateBegin = 0x4a,
    UpdateChunk = 0x4b,
    UpdateCommit = 0x4c,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0x01 => Self::DeviceHello,
            0x02 => Self::AuthResponse,
            0x03 => Self::InputEvent,
            0x04 => Self::InputSnapshot,
            0x05 => Self::DeviceStatus,
            0x06 => Self::Pong,
            0x07 => Self::Ack,
            0x08 => Self::ProtocolError,
            0x09 => Self::UpdateStatus,
            0x40 => Self::HostHello,
            0x41 => Self::AuthChallenge,
            0x42 => Self::ClaimBegin,
            0x43 => Self::ClaimCommit,
            0x44 => Self::GetInputSnapshot,
            0x45 => Self::GetDeviceStatus,
            0x46 => Self::VisualState,
            0x47 => Self::SetDeviceConfig,
            0x48 => Self::Ping,
            0x49 => Self::HostGoingAway,
            0x4a => Self::UpdateBegin,
            0x4b => Self::UpdateChunk,
            0x4c => Self::UpdateCommit,
            _ => return Err(ProtocolError::UnknownMessage(value)),
        })
    }
}

impl MessageType {
    pub fn direction(self) -> Direction {
        if (self as u8) < 0x40 {
            Direction::DeviceToHost
        } else {
            Direction::HostToDevice
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    DeviceToHost,
    HostToDevice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub message_type: MessageType,
    pub sequence: u32,
    pub sender_uptime_ms: u32,
    payload_length: u8,
    payload: [u8; PAYLOAD_CAPACITY],
}

impl Frame {
    pub fn new(
        message_type: MessageType,
        sequence: u32,
        sender_uptime_ms: u32,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        if sequence == 0 {
            return Err(ProtocolError::InvalidSequence);
        }
        if payload.len() > PAYLOAD_CAPACITY {
            return Err(ProtocolError::PayloadTooLarge(payload.len()));
        }
        validate_payload(message_type, payload)?;
        let mut bounded = [0; PAYLOAD_CAPACITY];
        bounded[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            message_type,
            sequence,
            sender_uptime_ms,
            payload_length: payload.len() as u8,
            payload: bounded,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.payload_length)]
    }

    pub fn encode(&self) -> [u8; FRAME_SIZE] {
        let mut bytes = [0; FRAME_SIZE];
        bytes[0] = 0xab;
        bytes[1] = PROTOCOL_MAJOR;
        bytes[2] = PROTOCOL_MINOR;
        bytes[3] = self.message_type as u8;
        bytes[5] = self.payload_length;
        bytes[6..10].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[10..14].copy_from_slice(&self.sender_uptime_ms.to_le_bytes());
        bytes[14..14 + self.payload().len()].copy_from_slice(self.payload());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != FRAME_SIZE {
            return Err(ProtocolError::InvalidFrameLength(bytes.len()));
        }
        if bytes[0] != 0xab {
            return Err(ProtocolError::InvalidMagic(bytes[0]));
        }
        if bytes[1] != PROTOCOL_MAJOR || bytes[2] > PROTOCOL_MINOR {
            return Err(ProtocolError::UnsupportedVersion(bytes[1], bytes[2]));
        }
        let message_type = MessageType::try_from(bytes[3])?;
        if bytes[4] != 0 {
            return Err(ProtocolError::InvalidFlags(bytes[4]));
        }
        let payload_length = usize::from(bytes[5]);
        if payload_length > PAYLOAD_CAPACITY {
            return Err(ProtocolError::PayloadTooLarge(payload_length));
        }
        let sequence = read_u32(&bytes[6..10]);
        if sequence == 0 {
            return Err(ProtocolError::InvalidSequence);
        }
        if bytes[14 + payload_length..62].iter().any(|byte| *byte != 0) {
            return Err(ProtocolError::NonCanonicalPadding);
        }
        if bytes[62] != 0 {
            return Err(ProtocolError::InvalidReserved);
        }
        let payload = &bytes[14..14 + payload_length];
        Self::new(message_type, sequence, read_u32(&bytes[10..14]), payload)
    }

    pub fn encode_usb(&self, direction: Direction) -> Result<[u8; USB_REPORT_SIZE], ProtocolError> {
        if self.message_type.direction() != direction {
            return Err(ProtocolError::WrongDirection);
        }
        let mut report = [0; USB_REPORT_SIZE];
        report[0] = match direction {
            Direction::DeviceToHost => USB_INPUT_REPORT_ID,
            Direction::HostToDevice => USB_OUTPUT_REPORT_ID,
        };
        report[1..].copy_from_slice(&self.encode());
        Ok(report)
    }

    pub fn decode_usb(report: &[u8], direction: Direction) -> Result<Self, ProtocolError> {
        if report.len() != USB_REPORT_SIZE {
            return Err(ProtocolError::InvalidUsbReportLength(report.len()));
        }
        let expected = match direction {
            Direction::DeviceToHost => USB_INPUT_REPORT_ID,
            Direction::HostToDevice => USB_OUTPUT_REPORT_ID,
        };
        if report[0] != expected {
            return Err(ProtocolError::InvalidUsbReportId(report[0]));
        }
        let frame = Self::decode(&report[1..])?;
        if frame.message_type.direction() != direction {
            return Err(ProtocolError::WrongDirection);
        }
        Ok(frame)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidFrameLength(usize),
    InvalidUsbReportLength(usize),
    InvalidUsbReportId(u8),
    InvalidMagic(u8),
    UnsupportedVersion(u8, u8),
    UnknownMessage(u8),
    InvalidFlags(u8),
    PayloadTooLarge(usize),
    InvalidSequence,
    NonCanonicalPadding,
    InvalidReserved,
    InvalidPayloadLength {
        message_type: MessageType,
        actual: usize,
    },
    InvalidPayload(&'static str),
    WrongDirection,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WireControlKind {
    Agent = 0,
    Action = 1,
    Joystick = 2,
    EncoderPress = 3,
    EncoderRotate = 4,
}

impl TryFrom<u8> for WireControlKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Agent),
            1 => Ok(Self::Action),
            2 => Ok(Self::Joystick),
            3 => Ok(Self::EncoderPress),
            4 => Ok(Self::EncoderRotate),
            _ => Err(ProtocolError::InvalidPayload("unknown control kind")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WirePhase {
    Pressed = 0,
    Released = 1,
    Entered = 2,
    Centered = 3,
    Rotated = 4,
}

impl TryFrom<u8> for WirePhase {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Pressed),
            1 => Ok(Self::Released),
            2 => Ok(Self::Entered),
            3 => Ok(Self::Centered),
            4 => Ok(Self::Rotated),
            _ => Err(ProtocolError::InvalidPayload("unknown input phase")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireInputEvent {
    pub boot_id: u32,
    pub control_kind: WireControlKind,
    pub control_id: u8,
    pub phase: WirePhase,
    pub value: i16,
}

impl WireInputEvent {
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        require_length(MessageType::InputEvent, payload, 9)?;
        let event = Self {
            boot_id: read_u32(&payload[0..4]),
            control_kind: WireControlKind::try_from(payload[4])?,
            control_id: payload[5],
            phase: WirePhase::try_from(payload[6])?,
            value: i16::from_le_bytes([payload[7], payload[8]]),
        };
        validate_wire_input(event)?;
        Ok(event)
    }

    pub fn encode(self) -> Result<[u8; 9], ProtocolError> {
        validate_wire_input(self)?;
        let mut payload = [0; 9];
        payload[0..4].copy_from_slice(&self.boot_id.to_le_bytes());
        payload[4] = self.control_kind as u8;
        payload[5] = self.control_id;
        payload[6] = self.phase as u8;
        payload[7..9].copy_from_slice(&self.value.to_le_bytes());
        Ok(payload)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireInputSnapshot {
    pub boot_id: u32,
    pub agent_mask: u8,
    pub action_mask: u8,
    pub joystick_mask: u8,
    pub encoder_pressed: bool,
}

impl WireInputSnapshot {
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        require_length(MessageType::InputSnapshot, payload, 8)?;
        let snapshot = Self {
            boot_id: read_u32(&payload[0..4]),
            agent_mask: payload[4],
            action_mask: payload[5],
            joystick_mask: payload[6],
            encoder_pressed: decode_bool(payload[7])?,
        };
        validate_snapshot(snapshot)?;
        Ok(snapshot)
    }

    pub fn encode(self) -> Result<[u8; 8], ProtocolError> {
        validate_snapshot(self)?;
        let mut payload = [0; 8];
        payload[0..4].copy_from_slice(&self.boot_id.to_le_bytes());
        payload[4] = self.agent_mask;
        payload[5] = self.action_mask;
        payload[6] = self.joystick_mask;
        payload[7] = u8::from(self.encoder_pressed);
        Ok(payload)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentVisualState {
    Empty = 0,
    Idle = 1,
    Thinking = 2,
    Complete = 3,
    RequiresInput = 4,
    Error = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VisualMicState {
    Idle = 0,
    Recording = 1,
    Processing = 2,
    Ready = 3,
    Locked = 4,
    Stopping = 5,
    Unknown = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VisualBridgeState {
    Offline = 0,
    Ready = 1,
    Degraded = 2,
    Incompatible = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VisualConnectionState {
    Offline = 0,
    Ready = 1,
    Pairing = 2,
    Claiming = 3,
    Updating = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VisualProfile {
    Usb = 0,
    Bt1 = 1,
    Bt2 = 2,
    Bt3 = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualState {
    pub revision: u32,
    pub agents: [AgentVisualState; 6],
    pub selected_mask: u8,
    pub mic: VisualMicState,
    pub bridge: VisualBridgeState,
    pub connection: VisualConnectionState,
    pub profile: VisualProfile,
    pub battery_percent: u8,
    pub power_flags: u8,
    pub brightness_percent: u8,
    pub auto_dim_seconds: u16,
}

impl VisualState {
    pub fn encode(self) -> Result<[u8; 20], ProtocolError> {
        validate_visual_state(&self)?;
        let mut payload = [0; 20];
        payload[0..4].copy_from_slice(&self.revision.to_le_bytes());
        for (target, state) in payload[4..10].iter_mut().zip(self.agents) {
            *target = state as u8;
        }
        payload[10] = self.selected_mask;
        payload[11] = self.mic as u8;
        payload[12] = self.bridge as u8;
        payload[13] = self.connection as u8;
        payload[14] = self.profile as u8;
        payload[15] = self.battery_percent;
        payload[16] = self.power_flags;
        payload[17] = self.brightness_percent;
        payload[18..20].copy_from_slice(&self.auto_dim_seconds.to_le_bytes());
        Ok(payload)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        require_length(MessageType::VisualState, payload, 20)?;
        let state = Self {
            revision: read_u32(&payload[0..4]),
            agents: [
                decode_agent_visual(payload[4])?,
                decode_agent_visual(payload[5])?,
                decode_agent_visual(payload[6])?,
                decode_agent_visual(payload[7])?,
                decode_agent_visual(payload[8])?,
                decode_agent_visual(payload[9])?,
            ],
            selected_mask: payload[10],
            mic: decode_mic(payload[11])?,
            bridge: decode_bridge(payload[12])?,
            connection: decode_connection(payload[13])?,
            profile: decode_profile(payload[14])?,
            battery_percent: payload[15],
            power_flags: payload[16],
            brightness_percent: payload[17],
            auto_dim_seconds: read_u16(&payload[18..20]),
        };
        validate_visual_state(&state)?;
        Ok(state)
    }
}

fn validate_payload(message_type: MessageType, payload: &[u8]) -> Result<(), ProtocolError> {
    use MessageType::*;
    match message_type {
        DeviceHello => {
            require_length(message_type, payload, 24)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            nonzero(&payload[4..20], "zero device ID")?;
        }
        AuthResponse => {
            require_length(message_type, payload, 36)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            nonzero(&payload[4..36], "zero auth response")?;
        }
        InputEvent => {
            WireInputEvent::decode(payload)?;
        }
        InputSnapshot => {
            WireInputSnapshot::decode(payload)?;
        }
        DeviceStatus => {
            require_length(message_type, payload, 10)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            range(payload[4], 3, "invalid connection mode")?;
            percent_or_unknown(payload[5])?;
            bits(payload[6], 0x07, "invalid power flags")?;
        }
        Pong => {
            require_length(message_type, payload, 8)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
        }
        Ack => {
            require_length(message_type, payload, 9)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            nonzero_u32(&payload[4..8], "zero acknowledged sequence")?;
            range(payload[8], 2, "invalid ack status")?;
        }
        ProtocolError => {
            require_length(message_type, payload, 10)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            range_between(payload[8], 1, 9, "invalid protocol error")?;
        }
        UpdateStatus => {
            require_length(message_type, payload, 8)?;
            nonzero_u32(&payload[0..4], "zero boot_id")?;
            range(payload[4], 5, "invalid update state")?;
            percent(payload[5])?;
            if payload[7] != 0 {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "non-zero update reserved",
                ));
            }
        }
        HostHello => {
            require_length(message_type, payload, 20)?;
            nonzero(&payload[0..16], "zero host nonce")?;
            if read_u32(&payload[16..20]) & !1 != 0 {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "unknown host feature",
                ));
            }
        }
        AuthChallenge | ClaimCommit | UpdateCommit => {
            require_length(message_type, payload, 32)?;
            nonzero(payload, "zero cryptographic value")?;
        }
        ClaimBegin => {
            require_length(message_type, payload, 48)?;
            nonzero(&payload[0..15], "zero claim nonce")?;
            if !matches!(payload[15], 0x02 | 0x03) {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "invalid P-256 key prefix",
                ));
            }
            nonzero(&payload[16..48], "zero P-256 X coordinate")?;
        }
        GetInputSnapshot | GetDeviceStatus => {
            require_length(message_type, payload, 0)?;
        }
        VisualState => {
            crate::protocol::VisualState::decode(payload)?;
        }
        SetDeviceConfig => {
            require_length(message_type, payload, 6)?;
            percent(payload[0])?;
            if read_u16(&payload[1..3]) > 3600 {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "invalid auto-dim",
                ));
            }
            range(payload[3], 3, "invalid profile")?;
            range(payload[4], 1, "invalid input mode")?;
            if payload[5] != 0 {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "non-zero config reserved",
                ));
            }
        }
        Ping => {
            require_length(message_type, payload, 4)?;
        }
        HostGoingAway => {
            require_length(message_type, payload, 1)?;
            range(payload[0], 3, "invalid going-away reason")?;
        }
        UpdateBegin => {
            require_length(message_type, payload, 44)?;
            nonzero_u32(&payload[0..4], "zero image size")?;
            bits(payload[9], 0x01, "invalid image flags")?;
            nonzero(&payload[10..42], "zero image digest")?;
            range_between_u16(read_u16(&payload[42..44]), 1, 43, "invalid chunk size")?;
        }
        UpdateChunk => {
            if !(5..=48).contains(&payload.len()) {
                return Err(crate::protocol::ProtocolError::InvalidPayloadLength {
                    message_type,
                    actual: payload.len(),
                });
            }
            let data_length = usize::from(payload[4]);
            if !(1..=43).contains(&data_length) || payload.len() != 5 + data_length {
                return Err(crate::protocol::ProtocolError::InvalidPayload(
                    "invalid update chunk length",
                ));
            }
        }
    }
    Ok(())
}

fn validate_wire_input(event: WireInputEvent) -> Result<(), ProtocolError> {
    if event.boot_id == 0 {
        return Err(ProtocolError::InvalidPayload("zero boot_id"));
    }
    let valid_id = match event.control_kind {
        WireControlKind::Agent => event.control_id <= 5,
        WireControlKind::Action => event.control_id <= 6,
        WireControlKind::Joystick => event.control_id <= 3,
        WireControlKind::EncoderPress | WireControlKind::EncoderRotate => event.control_id == 0,
    };
    if !valid_id {
        return Err(ProtocolError::InvalidPayload("invalid control ID"));
    }
    let valid_phase = matches!(
        (event.control_kind, event.phase),
        (
            WireControlKind::Agent | WireControlKind::Action | WireControlKind::EncoderPress,
            WirePhase::Pressed | WirePhase::Released
        ) | (
            WireControlKind::Joystick,
            WirePhase::Entered | WirePhase::Centered
        ) | (WireControlKind::EncoderRotate, WirePhase::Rotated)
    );
    if !valid_phase {
        return Err(ProtocolError::InvalidPayload("invalid control phase"));
    }
    if event.phase == WirePhase::Rotated {
        if event.value == 0 || event.value.unsigned_abs() > 32 {
            return Err(ProtocolError::InvalidPayload("invalid encoder steps"));
        }
    } else if event.value != 0 {
        return Err(ProtocolError::InvalidPayload("edge value must be zero"));
    }
    Ok(())
}

fn validate_snapshot(snapshot: WireInputSnapshot) -> Result<(), ProtocolError> {
    if snapshot.boot_id == 0 {
        return Err(ProtocolError::InvalidPayload("zero boot_id"));
    }
    bits(snapshot.agent_mask, 0x3f, "invalid Agent mask")?;
    bits(snapshot.action_mask, 0x7f, "invalid Action mask")?;
    bits(snapshot.joystick_mask, 0x0f, "invalid Joystick mask")?;
    if snapshot.joystick_mask.count_ones() > 1 {
        return Err(ProtocolError::InvalidPayload(
            "multiple joystick directions",
        ));
    }
    Ok(())
}

fn validate_visual_state(state: &VisualState) -> Result<(), ProtocolError> {
    bits(state.selected_mask, 0x3f, "invalid selected mask")?;
    percent_or_unknown(state.battery_percent)?;
    bits(state.power_flags, 0x07, "invalid power flags")?;
    percent(state.brightness_percent)?;
    if state.auto_dim_seconds > 3600 {
        return Err(ProtocolError::InvalidPayload("invalid auto-dim"));
    }
    Ok(())
}

fn decode_agent_visual(value: u8) -> Result<AgentVisualState, ProtocolError> {
    Ok(match value {
        0 => AgentVisualState::Empty,
        1 => AgentVisualState::Idle,
        2 => AgentVisualState::Thinking,
        3 => AgentVisualState::Complete,
        4 => AgentVisualState::RequiresInput,
        5 => AgentVisualState::Error,
        _ => return Err(ProtocolError::InvalidPayload("invalid Agent visual state")),
    })
}

fn decode_mic(value: u8) -> Result<VisualMicState, ProtocolError> {
    Ok(match value {
        0 => VisualMicState::Idle,
        1 => VisualMicState::Recording,
        2 => VisualMicState::Processing,
        3 => VisualMicState::Ready,
        4 => VisualMicState::Locked,
        5 => VisualMicState::Stopping,
        6 => VisualMicState::Unknown,
        _ => return Err(ProtocolError::InvalidPayload("invalid Mic state")),
    })
}

fn decode_bridge(value: u8) -> Result<VisualBridgeState, ProtocolError> {
    Ok(match value {
        0 => VisualBridgeState::Offline,
        1 => VisualBridgeState::Ready,
        2 => VisualBridgeState::Degraded,
        3 => VisualBridgeState::Incompatible,
        _ => return Err(ProtocolError::InvalidPayload("invalid Bridge state")),
    })
}

fn decode_connection(value: u8) -> Result<VisualConnectionState, ProtocolError> {
    Ok(match value {
        0 => VisualConnectionState::Offline,
        1 => VisualConnectionState::Ready,
        2 => VisualConnectionState::Pairing,
        3 => VisualConnectionState::Claiming,
        4 => VisualConnectionState::Updating,
        _ => return Err(ProtocolError::InvalidPayload("invalid connection state")),
    })
}

fn decode_profile(value: u8) -> Result<VisualProfile, ProtocolError> {
    Ok(match value {
        0 => VisualProfile::Usb,
        1 => VisualProfile::Bt1,
        2 => VisualProfile::Bt2,
        3 => VisualProfile::Bt3,
        _ => return Err(ProtocolError::InvalidPayload("invalid profile")),
    })
}

fn require_length(
    message_type: MessageType,
    payload: &[u8],
    expected: usize,
) -> Result<(), ProtocolError> {
    if payload.len() != expected {
        return Err(ProtocolError::InvalidPayloadLength {
            message_type,
            actual: payload.len(),
        });
    }
    Ok(())
}

fn nonzero(bytes: &[u8], reason: &'static str) -> Result<(), ProtocolError> {
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn nonzero_u32(bytes: &[u8], reason: &'static str) -> Result<(), ProtocolError> {
    if read_u32(bytes) == 0 {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn decode_bool(value: u8) -> Result<bool, ProtocolError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::InvalidPayload("invalid boolean")),
    }
}

fn percent(value: u8) -> Result<(), ProtocolError> {
    range(value, 100, "invalid percent")
}

fn percent_or_unknown(value: u8) -> Result<(), ProtocolError> {
    if value == 255 {
        Ok(())
    } else {
        percent(value)
    }
}

fn range(value: u8, maximum: u8, reason: &'static str) -> Result<(), ProtocolError> {
    if value > maximum {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn range_between(
    value: u8,
    minimum: u8,
    maximum: u8,
    reason: &'static str,
) -> Result<(), ProtocolError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn range_between_u16(
    value: u16,
    minimum: u16,
    maximum: u16,
    reason: &'static str,
) -> Result<(), ProtocolError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn bits(value: u8, allowed: u8, reason: &'static str) -> Result<(), ProtocolError> {
    if value & !allowed != 0 {
        return Err(ProtocolError::InvalidPayload(reason));
    }
    Ok(())
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::{fs, path::PathBuf};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Manifest {
        schema_version: u8,
        protocol_version: String,
        frame_size: usize,
        packets: Vec<GoldenEntry>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenEntry {
        name: String,
        type_id: u8,
        sequence: u32,
        sender_uptime_ms: u32,
        payload_hex: String,
    }

    fn golden_directory() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/golden")
    }

    fn manifest() -> Manifest {
        serde_json::from_slice(
            &fs::read(golden_directory().join("manifest.json")).expect("golden manifest"),
        )
        .expect("valid golden manifest")
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0);
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn every_golden_packet_round_trips_byte_for_byte() {
        let manifest = manifest();
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.protocol_version, "1.0");
        assert_eq!(manifest.frame_size, FRAME_SIZE);
        assert_eq!(manifest.packets.len(), 22);
        for entry in manifest.packets {
            let bytes = fs::read(golden_directory().join(format!("{}.bin", entry.name))).unwrap();
            let decoded = Frame::decode(&bytes).unwrap_or_else(|error| {
                panic!("{} must decode: {error}", entry.name);
            });
            assert_eq!(decoded.message_type as u8, entry.type_id, "{}", entry.name);
            assert_eq!(decoded.sequence, entry.sequence, "{}", entry.name);
            assert_eq!(
                decoded.sender_uptime_ms, entry.sender_uptime_ms,
                "{}",
                entry.name
            );
            assert_eq!(
                decoded.payload(),
                decode_hex(&entry.payload_hex),
                "{}",
                entry.name
            );
            assert_eq!(decoded.encode().as_slice(), bytes, "{}", entry.name);
        }
    }

    #[test]
    fn truncation_oversize_and_header_mutations_fail_closed() {
        let valid = fs::read(golden_directory().join("input_event_ag00_pressed.bin")).unwrap();
        for length in 0..FRAME_SIZE {
            assert!(Frame::decode(&valid[..length]).is_err(), "length {length}");
        }
        let mut oversized = valid.clone();
        oversized.push(0);
        assert!(Frame::decode(&oversized).is_err());

        for (offset, value) in [(0, 0), (1, 2), (2, 1), (3, 0xff), (4, 1), (5, 49), (62, 1)] {
            let mut malformed = valid.clone();
            malformed[offset] = value;
            assert!(Frame::decode(&malformed).is_err(), "offset {offset}");
        }
        let mut padding = valid;
        padding[61] = 1;
        assert!(Frame::decode(&padding).is_err());
    }

    #[test]
    fn checked_in_malformed_corpus_is_rejected() {
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fuzz/corpus");
        let mut count = 0;
        for entry in fs::read_dir(corpus).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "bin") {
                let bytes = fs::read(&path).unwrap();
                assert!(Frame::decode(&bytes).is_err(), "{}", path.display());
                count += 1;
            }
        }
        assert_eq!(count, 95);
    }

    #[test]
    fn invalid_control_phase_value_and_payload_enum_fail_closed() {
        let base = fs::read(golden_directory().join("input_event_ag00_pressed.bin")).unwrap();
        for (payload_offset, value) in [(4, 9), (5, 6), (6, 4), (7, 1)] {
            let mut malformed = base.clone();
            malformed[14 + payload_offset] = value;
            assert!(Frame::decode(&malformed).is_err());
        }

        for (name, payload_offset) in [
            ("device_status", 4usize),
            ("visual_state", 19usize),
            ("set_device_config", 5usize),
            ("update_status", 7usize),
        ] {
            let mut malformed = fs::read(golden_directory().join(format!("{name}.bin"))).unwrap();
            malformed[14 + payload_offset] = 0xff;
            assert!(Frame::decode(&malformed).is_err(), "{name}");
        }
    }

    #[test]
    fn every_control_id_phase_and_boundary_value_matches_the_closed_schema() {
        let expected = |kind: u8, id: u8, phase: u8, value: i16| {
            let valid_id = matches!(
                (kind, id),
                (0, 0..=5) | (1, 0..=6) | (2, 0..=3) | (3 | 4, 0)
            );
            let valid_phase = matches!((kind, phase), (0 | 1 | 3, 0 | 1) | (2, 2 | 3) | (4, 4));
            let valid_value = if phase == 4 {
                value != 0 && (-32..=32).contains(&value)
            } else {
                value == 0
            };
            valid_id && valid_phase && valid_value
        };

        for kind in 0..=5 {
            for id in 0..=7 {
                for phase in 0..=5 {
                    for value in [-33i16, -32, -1, 0, 1, 32, 33] {
                        let mut bytes = [0u8; FRAME_SIZE];
                        bytes[0] = 0xab;
                        bytes[1] = PROTOCOL_MAJOR;
                        bytes[3] = MessageType::InputEvent as u8;
                        bytes[5] = 9;
                        bytes[6] = 1;
                        bytes[14] = 1;
                        bytes[18] = kind;
                        bytes[19] = id;
                        bytes[20] = phase;
                        bytes[21..23].copy_from_slice(&value.to_le_bytes());
                        assert_eq!(
                            Frame::decode(&bytes).is_ok(),
                            expected(kind, id, phase, value),
                            "kind={kind} id={id} phase={phase} value={value}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn deterministic_random_frames_never_panic_or_escape_canonical_validation() {
        let mut state = 0x5eed_cafe_u32;
        for _ in 0..20_000 {
            let mut bytes = [0; FRAME_SIZE];
            for byte in &mut bytes {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *byte = (state >> 24) as u8;
            }
            if let Ok(frame) = Frame::decode(&bytes) {
                assert_eq!(frame.encode(), bytes);
            }
        }
    }

    #[test]
    fn usb_wrapper_checks_report_id_length_and_direction() {
        let frame = Frame::new(MessageType::Ping, 1, 0, &[1, 2, 3, 4]).unwrap();
        let report = frame.encode_usb(Direction::HostToDevice).unwrap();
        assert_eq!(
            Frame::decode_usb(&report, Direction::HostToDevice).unwrap(),
            frame
        );
        assert!(Frame::decode_usb(&report[..63], Direction::HostToDevice).is_err());
        assert!(Frame::decode_usb(&report, Direction::DeviceToHost).is_err());
    }
}
