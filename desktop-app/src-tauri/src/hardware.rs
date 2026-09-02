use std::collections::{HashMap, HashSet};

#[cfg(test)]
use std::collections::VecDeque;

use crate::{
    input::InputCoordinator,
    model::{
        AgentVisualStatus, AppSnapshot, BridgeState, CommandSlot, ControlId, Direction,
        HardwareTransport, InputEvent, InputPhase, MicState, TrustedHardwareSource,
    },
    protocol::{
        AgentVisualState, Direction as ProtocolDirection, Frame, MessageType, ProtocolError,
        VisualBridgeState, VisualConnectionState, VisualMicState, VisualProfile, VisualState,
        WireControlKind, WireInputEvent, WireInputSnapshot, WirePhase, FRAME_SIZE, USB_REPORT_SIZE,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConnectionToken {
    source_id: u64,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionRejection {
    Malformed,
    WrongDirection,
    AwaitingHello,
    AwaitingSnapshot,
    Duplicate,
    OutOfOrder,
    Gap,
    SequenceExhausted,
    BootMismatch,
    UnexpectedMessage,
    StaleGeneration,
}

#[derive(Debug, Default)]
pub(crate) struct ControllerOutcome {
    pub(crate) events: Vec<InputEvent>,
    pub(crate) outbound: Vec<[u8; FRAME_SIZE]>,
    pub(crate) rejection: Option<SessionRejection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    AwaitingHello,
    AwaitingSnapshot { boot_id: u32 },
    Ready { boot_id: u32 },
}

#[derive(Debug)]
struct DeviceSession {
    state: SessionState,
    last_device_sequence: Option<u32>,
    next_host_sequence: u32,
    retired_boot_id: Option<u32>,
}

#[derive(Debug)]
enum SessionEvent {
    Hello { rebooted: bool },
    Snapshot(WireInputSnapshot),
    Input(WireInputEvent),
    Ignored,
}

#[derive(Debug)]
struct SessionFailure {
    rejection: SessionRejection,
    requires_snapshot: bool,
}

impl DeviceSession {
    fn new() -> Self {
        Self {
            state: SessionState::AwaitingHello,
            last_device_sequence: None,
            next_host_sequence: 1,
            retired_boot_id: None,
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, SessionState::Ready { .. })
    }

    fn accept(&mut self, frame: &Frame) -> Result<SessionEvent, SessionFailure> {
        if frame.message_type.direction() != crate::protocol::Direction::DeviceToHost {
            return Err(SessionFailure {
                rejection: SessionRejection::WrongDirection,
                requires_snapshot: false,
            });
        }

        if frame.message_type == MessageType::DeviceHello {
            let boot_id = u32::from_le_bytes(frame.payload()[0..4].try_into().unwrap());
            let current_boot = match self.state {
                SessionState::AwaitingHello => None,
                SessionState::AwaitingSnapshot { boot_id } | SessionState::Ready { boot_id } => {
                    Some(boot_id)
                }
            };
            if current_boot.is_some_and(|current| current != boot_id) {
                if self.retired_boot_id == Some(boot_id) || frame.sequence != 1 {
                    return Err(SessionFailure {
                        rejection: SessionRejection::BootMismatch,
                        requires_snapshot: false,
                    });
                }
                self.retired_boot_id = current_boot;
                self.state = SessionState::AwaitingSnapshot { boot_id };
                self.last_device_sequence = Some(1);
                return Ok(SessionEvent::Hello { rebooted: true });
            }
        }

        self.accept_sequence(frame.sequence)?;
        match frame.message_type {
            MessageType::DeviceHello => {
                if self.state != SessionState::AwaitingHello || frame.sequence != 1 {
                    return Err(SessionFailure {
                        rejection: SessionRejection::UnexpectedMessage,
                        requires_snapshot: false,
                    });
                }
                let boot_id = u32::from_le_bytes(frame.payload()[0..4].try_into().unwrap());
                self.state = SessionState::AwaitingSnapshot { boot_id };
                Ok(SessionEvent::Hello { rebooted: false })
            }
            MessageType::InputSnapshot => {
                let snapshot =
                    WireInputSnapshot::decode(frame.payload()).map_err(|_| SessionFailure {
                        rejection: SessionRejection::Malformed,
                        requires_snapshot: false,
                    })?;
                match self.state {
                    SessionState::AwaitingSnapshot { boot_id } if boot_id == snapshot.boot_id => {
                        self.state = SessionState::Ready { boot_id };
                        Ok(SessionEvent::Snapshot(snapshot))
                    }
                    SessionState::AwaitingSnapshot { .. } => Err(SessionFailure {
                        rejection: SessionRejection::BootMismatch,
                        requires_snapshot: true,
                    }),
                    SessionState::Ready { boot_id } => {
                        self.state = SessionState::AwaitingSnapshot { boot_id };
                        Err(SessionFailure {
                            rejection: SessionRejection::BootMismatch,
                            requires_snapshot: true,
                        })
                    }
                    SessionState::AwaitingHello => Err(SessionFailure {
                        rejection: SessionRejection::AwaitingHello,
                        requires_snapshot: false,
                    }),
                }
            }
            MessageType::InputEvent => {
                let event =
                    WireInputEvent::decode(frame.payload()).map_err(|_| SessionFailure {
                        rejection: SessionRejection::Malformed,
                        requires_snapshot: false,
                    })?;
                match self.state {
                    SessionState::Ready { boot_id } if boot_id == event.boot_id => {
                        Ok(SessionEvent::Input(event))
                    }
                    SessionState::Ready { boot_id } => {
                        self.state = SessionState::AwaitingSnapshot { boot_id };
                        Err(SessionFailure {
                            rejection: SessionRejection::BootMismatch,
                            requires_snapshot: true,
                        })
                    }
                    SessionState::AwaitingHello => Err(SessionFailure {
                        rejection: SessionRejection::AwaitingHello,
                        requires_snapshot: false,
                    }),
                    SessionState::AwaitingSnapshot { .. } => Err(SessionFailure {
                        rejection: SessionRejection::AwaitingSnapshot,
                        requires_snapshot: false,
                    }),
                }
            }
            MessageType::AuthResponse
            | MessageType::DeviceStatus
            | MessageType::Pong
            | MessageType::Ack
            | MessageType::ProtocolError
            | MessageType::UpdateStatus => Ok(SessionEvent::Ignored),
            _ => Err(SessionFailure {
                rejection: SessionRejection::WrongDirection,
                requires_snapshot: false,
            }),
        }
    }

    fn accept_sequence(&mut self, sequence: u32) -> Result<(), SessionFailure> {
        if self.last_device_sequence == Some(u32::MAX) {
            return Err(SessionFailure {
                rejection: SessionRejection::SequenceExhausted,
                requires_snapshot: false,
            });
        }
        let expected = self.last_device_sequence.map_or(1, |last| last + 1);
        if sequence == expected {
            self.last_device_sequence = Some(sequence);
            return Ok(());
        }
        if self
            .last_device_sequence
            .is_some_and(|last| sequence == last)
        {
            return Err(SessionFailure {
                rejection: SessionRejection::Duplicate,
                requires_snapshot: false,
            });
        }
        if self
            .last_device_sequence
            .is_some_and(|last| sequence < last)
        {
            return Err(SessionFailure {
                rejection: SessionRejection::OutOfOrder,
                requires_snapshot: false,
            });
        }
        self.last_device_sequence = Some(sequence);
        if let SessionState::Ready { boot_id } = self.state {
            self.state = SessionState::AwaitingSnapshot { boot_id };
        }
        Err(SessionFailure {
            rejection: SessionRejection::Gap,
            requires_snapshot: true,
        })
    }

    fn host_frame(
        &mut self,
        message_type: MessageType,
        payload: &[u8],
    ) -> Result<[u8; FRAME_SIZE], ProtocolError> {
        let sequence = self.next_host_sequence;
        self.next_host_sequence = self
            .next_host_sequence
            .checked_add(1)
            .ok_or(ProtocolError::InvalidSequence)?;
        Ok(Frame::new(message_type, sequence, 0, payload)?.encode())
    }
}

#[derive(Debug)]
struct ConnectedSession {
    source: TrustedHardwareSource,
    session: DeviceSession,
}

#[derive(Debug, Default)]
pub(crate) struct HardwareController {
    sessions: HashMap<ConnectionToken, ConnectedSession>,
    current_generation: HashMap<u64, u64>,
    next_source_id: u64,
    next_generation: u64,
}

impl HardwareController {
    pub(crate) fn is_ready(&self, connection: ConnectionToken) -> bool {
        self.sessions
            .get(&connection)
            .is_some_and(|connected| connected.session.is_ready())
    }

    pub(crate) fn attach(
        &mut self,
        transport: HardwareTransport,
        inputs: &mut InputCoordinator,
    ) -> ConnectionToken {
        self.next_source_id += 1;
        self.attach_source(self.next_source_id, transport, inputs)
    }

    #[cfg(test)]
    pub(crate) fn reconnect(
        &mut self,
        previous: ConnectionToken,
        transport: HardwareTransport,
        inputs: &mut InputCoordinator,
    ) -> (ConnectionToken, Vec<InputEvent>) {
        let releases = self.disconnect(previous, inputs);
        (
            self.attach_source(previous.source_id, transport, inputs),
            releases,
        )
    }

    fn attach_source(
        &mut self,
        source_id: u64,
        transport: HardwareTransport,
        inputs: &mut InputCoordinator,
    ) -> ConnectionToken {
        self.next_generation += 1;
        let token = ConnectionToken {
            source_id,
            generation: self.next_generation,
        };
        let source = TrustedHardwareSource {
            id: source_id,
            transport,
            generation: token.generation,
        };
        self.current_generation.insert(source_id, token.generation);
        inputs.register_hardware(source);
        self.sessions.insert(
            token,
            ConnectedSession {
                source,
                session: DeviceSession::new(),
            },
        );
        token
    }

    pub(crate) fn receive(
        &mut self,
        connection: ConnectionToken,
        bytes: &[u8],
        inputs: &mut InputCoordinator,
    ) -> ControllerOutcome {
        if self.current_generation.get(&connection.source_id) != Some(&connection.generation) {
            return ControllerOutcome {
                rejection: Some(SessionRejection::StaleGeneration),
                ..ControllerOutcome::default()
            };
        }
        let frame = match Frame::decode(bytes) {
            Ok(frame) => frame,
            Err(_) => {
                return ControllerOutcome {
                    rejection: Some(SessionRejection::Malformed),
                    ..ControllerOutcome::default()
                };
            }
        };
        let Some(connected) = self.sessions.get_mut(&connection) else {
            return ControllerOutcome {
                rejection: Some(SessionRejection::StaleGeneration),
                ..ControllerOutcome::default()
            };
        };
        let source = connected.source;
        match connected.session.accept(&frame) {
            Ok(SessionEvent::Hello { rebooted }) => {
                let mut outcome = ControllerOutcome::default();
                if rebooted {
                    outcome.events = inputs.desynchronize_hardware(source);
                }
                outcome.outbound.push(
                    connected
                        .session
                        .host_frame(MessageType::GetInputSnapshot, &[])
                        .expect("fixed snapshot request is valid"),
                );
                outcome
            }
            Ok(SessionEvent::Snapshot(snapshot)) => {
                let pressed = snapshot_controls(snapshot);
                match inputs.synchronize_hardware(source, frame.sequence, pressed) {
                    Ok(()) => ControllerOutcome::default(),
                    Err(_) => ControllerOutcome {
                        rejection: Some(SessionRejection::StaleGeneration),
                        ..ControllerOutcome::default()
                    },
                }
            }
            Ok(SessionEvent::Input(event)) => {
                let (control, phase) = model_input(event);
                match inputs.handle_hardware(
                    source,
                    frame.sequence,
                    f64::from(frame.sender_uptime_ms),
                    control,
                    phase,
                ) {
                    Ok(events) => ControllerOutcome {
                        events,
                        ..ControllerOutcome::default()
                    },
                    Err(_) => ControllerOutcome {
                        rejection: Some(SessionRejection::Malformed),
                        ..ControllerOutcome::default()
                    },
                }
            }
            Ok(SessionEvent::Ignored) => match inputs.observe_hardware(source, frame.sequence) {
                Ok(()) => ControllerOutcome::default(),
                Err(_) => ControllerOutcome {
                    rejection: Some(SessionRejection::Malformed),
                    ..ControllerOutcome::default()
                },
            },
            Err(failure) => {
                let mut outcome = ControllerOutcome {
                    rejection: Some(failure.rejection),
                    ..ControllerOutcome::default()
                };
                if failure.requires_snapshot {
                    outcome.events = inputs.desynchronize_hardware(source);
                    outcome.outbound.push(
                        connected
                            .session
                            .host_frame(MessageType::GetInputSnapshot, &[])
                            .expect("fixed snapshot request is valid"),
                    );
                }
                outcome
            }
        }
    }

    pub(crate) fn receive_usb(
        &mut self,
        connection: ConnectionToken,
        report: &[u8],
        inputs: &mut InputCoordinator,
    ) -> ControllerOutcome {
        let frame = match Frame::decode_usb(report, ProtocolDirection::DeviceToHost) {
            Ok(frame) => frame,
            Err(_) => {
                return ControllerOutcome {
                    rejection: Some(SessionRejection::Malformed),
                    ..ControllerOutcome::default()
                };
            }
        };
        self.receive(connection, &frame.encode(), inputs)
    }

    pub(crate) fn disconnect(
        &mut self,
        connection: ConnectionToken,
        inputs: &mut InputCoordinator,
    ) -> Vec<InputEvent> {
        let Some(connected) = self.sessions.remove(&connection) else {
            return Vec::new();
        };
        if self.current_generation.get(&connection.source_id) == Some(&connection.generation) {
            self.current_generation.remove(&connection.source_id);
        }
        inputs.disconnect_hardware(connected.source)
    }

    pub(crate) fn publish_visual_state(
        &mut self,
        state: HardwareVisualState,
    ) -> Vec<TransportWrite> {
        let payload = state
            .wire
            .encode()
            .expect("App-derived visual state is always valid");
        let mut writes = Vec::new();
        for (connection, connected) in &mut self.sessions {
            if connected.session.is_ready() {
                writes.push(TransportWrite {
                    connection: *connection,
                    bytes: connected
                        .session
                        .host_frame(MessageType::VisualState, &payload)
                        .expect("fixed visual payload is valid"),
                });
            }
        }
        writes
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TransportWrite {
    pub(crate) connection: ConnectionToken,
    pub(crate) bytes: [u8; FRAME_SIZE],
}

impl TransportWrite {
    pub(crate) fn usb_report(self) -> Result<[u8; USB_REPORT_SIZE], ProtocolError> {
        Frame::decode(&self.bytes)?.encode_usb(ProtocolDirection::HostToDevice)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HardwareVisualState {
    wire: VisualState,
}

impl HardwareVisualState {
    pub(crate) fn from_app_snapshot(snapshot: &AppSnapshot, revision: u32) -> Self {
        let mut agents = [AgentVisualState::Empty; 6];
        let mut selected_mask = 0;
        for agent in &snapshot.agents {
            let Some(target) = agents.get_mut(usize::from(agent.slot)) else {
                continue;
            };
            *target = match agent.status {
                AgentVisualStatus::Empty => AgentVisualState::Empty,
                AgentVisualStatus::Idle => AgentVisualState::Idle,
                AgentVisualStatus::Thinking => AgentVisualState::Thinking,
                AgentVisualStatus::Complete => AgentVisualState::Complete,
                AgentVisualStatus::Input => AgentVisualState::RequiresInput,
                AgentVisualStatus::Error => AgentVisualState::Error,
            };
            if agent.selected {
                selected_mask |= 1 << agent.slot;
            }
        }
        let bridge = match snapshot.bridge.state {
            BridgeState::Ready => VisualBridgeState::Ready,
            BridgeState::Degraded => VisualBridgeState::Degraded,
            BridgeState::Incompatible => VisualBridgeState::Incompatible,
            _ => VisualBridgeState::Offline,
        };
        Self {
            wire: VisualState {
                revision,
                agents,
                selected_mask,
                mic: match snapshot.mic_state.unwrap_or(MicState::Unknown) {
                    MicState::Idle => VisualMicState::Idle,
                    MicState::Recording => VisualMicState::Recording,
                    MicState::Processing => VisualMicState::Processing,
                    MicState::Ready => VisualMicState::Ready,
                    MicState::Locked => VisualMicState::Locked,
                    MicState::Stopping => VisualMicState::Stopping,
                    MicState::Unknown => VisualMicState::Unknown,
                },
                bridge,
                connection: VisualConnectionState::Ready,
                profile: VisualProfile::Usb,
                battery_percent: 255,
                power_flags: 0x02,
                brightness_percent: 50,
                auto_dim_seconds: 180,
            },
        }
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FakeTransport {
    connection: ConnectionToken,
    to_host: VecDeque<[u8; FRAME_SIZE]>,
    to_device: VecDeque<[u8; FRAME_SIZE]>,
    trace: Vec<MessageType>,
}

#[cfg(test)]
impl FakeTransport {
    pub(crate) fn new(connection: ConnectionToken) -> Self {
        Self {
            connection,
            to_host: VecDeque::new(),
            to_device: VecDeque::new(),
            trace: Vec::new(),
        }
    }

    pub(crate) fn send_from_device(&mut self, frame: [u8; FRAME_SIZE]) {
        self.trace.push(Frame::decode(&frame).unwrap().message_type);
        self.to_host.push_back(frame);
    }

    pub(crate) fn deliver_to_controller(
        &mut self,
        controller: &mut HardwareController,
        inputs: &mut InputCoordinator,
    ) -> Option<ControllerOutcome> {
        let frame = self.to_host.pop_front()?;
        let outcome = controller.receive(self.connection, &frame, inputs);
        for outbound in &outcome.outbound {
            self.trace
                .push(Frame::decode(outbound).unwrap().message_type);
            self.to_device.push_back(*outbound);
        }
        Some(outcome)
    }

    pub(crate) fn send_from_controller(&mut self, write: TransportWrite) {
        if write.connection == self.connection {
            self.trace
                .push(Frame::decode(&write.bytes).unwrap().message_type);
            self.to_device.push_back(write.bytes);
        }
    }

    pub(crate) fn receive_on_device(&mut self) -> Option<[u8; FRAME_SIZE]> {
        self.to_device.pop_front()
    }
}

fn model_input(event: WireInputEvent) -> (ControlId, InputPhase) {
    let control = match event.control_kind {
        WireControlKind::Agent => ControlId::Agent {
            slot: event.control_id,
        },
        WireControlKind::Action => ControlId::Command {
            slot: match event.control_id {
                0 => CommandSlot::Act06,
                1 => CommandSlot::Act07,
                2 => CommandSlot::Act08,
                3 => CommandSlot::Act09,
                4 | 5 => CommandSlot::Act10Act11,
                6 => CommandSlot::Act12,
                _ => unreachable!("wire validation bounds Action IDs"),
            },
        },
        WireControlKind::Joystick => ControlId::Joystick {
            direction: match event.control_id {
                0 => Direction::Up,
                1 => Direction::Right,
                2 => Direction::Down,
                3 => Direction::Left,
                _ => unreachable!("wire validation bounds Joystick IDs"),
            },
        },
        WireControlKind::EncoderPress => ControlId::EncoderPress,
        WireControlKind::EncoderRotate => ControlId::EncoderRotate,
    };
    let phase = match event.phase {
        WirePhase::Pressed => InputPhase::Pressed,
        WirePhase::Released => InputPhase::Released,
        WirePhase::Entered => InputPhase::Entered,
        WirePhase::Centered => InputPhase::Centered,
        WirePhase::Rotated => InputPhase::Rotated { steps: event.value },
    };
    (control, phase)
}

fn snapshot_controls(snapshot: WireInputSnapshot) -> HashSet<ControlId> {
    let mut controls = HashSet::new();
    for slot in 0..6 {
        if snapshot.agent_mask & (1 << slot) != 0 {
            controls.insert(ControlId::Agent { slot });
        }
    }
    for id in 0..7 {
        if snapshot.action_mask & (1 << id) != 0 {
            controls.insert(
                model_input(WireInputEvent {
                    boot_id: snapshot.boot_id,
                    control_kind: WireControlKind::Action,
                    control_id: id,
                    phase: WirePhase::Pressed,
                    value: 0,
                })
                .0,
            );
        }
    }
    for id in 0..4 {
        if snapshot.joystick_mask & (1 << id) != 0 {
            controls.insert(
                model_input(WireInputEvent {
                    boot_id: snapshot.boot_id,
                    control_kind: WireControlKind::Joystick,
                    control_id: id,
                    phase: WirePhase::Entered,
                    value: 0,
                })
                .0,
            );
        }
    }
    if snapshot.encoder_pressed {
        controls.insert(ControlId::EncoderPress);
    }
    controls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        control::ControlEngine,
        model::{AgentSlot, AppConfig, DispatchOutcome, DispatchReceipt, Intent, Layout},
    };

    struct FakeDevice {
        boot_id: u32,
        next_sequence: u32,
        visual_states: Vec<VisualState>,
    }

    impl FakeDevice {
        fn new(boot_id: u32) -> Self {
            Self {
                boot_id,
                next_sequence: 1,
                visual_states: Vec::new(),
            }
        }

        fn hello(&mut self) -> [u8; FRAME_SIZE] {
            let mut payload = [0; 24];
            payload[0..4].copy_from_slice(&self.boot_id.to_le_bytes());
            payload[4] = 1;
            payload[20..22].copy_from_slice(&1u16.to_le_bytes());
            payload[22] = 1;
            self.frame(MessageType::DeviceHello, &payload)
        }

        fn snapshot(&mut self, agent_mask: u8) -> [u8; FRAME_SIZE] {
            let payload = WireInputSnapshot {
                boot_id: self.boot_id,
                agent_mask,
                action_mask: 0,
                joystick_mask: 0,
                encoder_pressed: false,
            }
            .encode()
            .unwrap();
            self.frame(MessageType::InputSnapshot, &payload)
        }

        fn agent(&mut self, pressed: bool) -> [u8; FRAME_SIZE] {
            let payload = WireInputEvent {
                boot_id: self.boot_id,
                control_kind: WireControlKind::Agent,
                control_id: 0,
                phase: if pressed {
                    WirePhase::Pressed
                } else {
                    WirePhase::Released
                },
                value: 0,
            }
            .encode()
            .unwrap();
            self.frame(MessageType::InputEvent, &payload)
        }

        fn status(&mut self) -> [u8; FRAME_SIZE] {
            let mut payload = [0; 10];
            payload[0..4].copy_from_slice(&self.boot_id.to_le_bytes());
            payload[5] = 255;
            self.frame(MessageType::DeviceStatus, &payload)
        }

        fn frame(&mut self, message_type: MessageType, payload: &[u8]) -> [u8; FRAME_SIZE] {
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            Frame::new(message_type, sequence, sequence * 10, payload)
                .unwrap()
                .encode()
        }

        fn receive(&mut self, bytes: &[u8]) {
            let frame = Frame::decode(bytes).unwrap();
            if frame.message_type == MessageType::VisualState {
                self.visual_states
                    .push(VisualState::decode(frame.payload()).unwrap());
            }
        }
    }

    #[derive(Default)]
    struct FakeRenderer {
        intents: Vec<Intent>,
        receipts: Vec<DispatchReceipt>,
    }

    impl FakeRenderer {
        fn dispatch(&mut self, intent: Intent) -> DispatchReceipt {
            self.intents.push(intent.clone());
            let receipt = DispatchReceipt {
                request_id: format!("fake-{}", self.receipts.len() + 1),
                intent_kind: intent.kind().into(),
                outcome: DispatchOutcome::Executed,
                duration_ms: 0,
                capability: Some("fakeRenderer".into()),
                error_code: None,
            };
            self.receipts.push(receipt.clone());
            receipt
        }
    }

    fn establish(
        controller: &mut HardwareController,
        inputs: &mut InputCoordinator,
        transport: &mut FakeTransport,
        device: &mut FakeDevice,
    ) {
        transport.send_from_device(device.hello());
        let hello = transport.deliver_to_controller(controller, inputs).unwrap();
        assert!(hello.rejection.is_none());
        device.receive(&transport.receive_on_device().unwrap());
        transport.send_from_device(device.snapshot(0));
        let snapshot = transport.deliver_to_controller(controller, inputs).unwrap();
        assert!(snapshot.rejection.is_none());
    }

    #[test]
    fn fake_ag00_and_visual_state_round_trip_through_trusted_boundary() {
        let mut controller = HardwareController::default();
        let mut inputs = InputCoordinator::default();
        let connection = controller.attach(HardwareTransport::Fake, &mut inputs);
        let mut transport = FakeTransport::new(connection);
        let mut device = FakeDevice::new(0x1122_3344);
        establish(&mut controller, &mut inputs, &mut transport, &mut device);

        let layout = Layout::official_defaults();
        let agents: Vec<_> = (0..6).map(AgentSlot::empty).collect();
        let mut engine = ControlEngine::default();
        let mut renderer = FakeRenderer::default();
        for pressed in [true, false] {
            transport.send_from_device(device.agent(pressed));
            let outcome = transport
                .deliver_to_controller(&mut controller, &mut inputs)
                .unwrap();
            assert!(outcome.rejection.is_none());
            for event in outcome.events {
                for intent in engine.handle(event, &layout, &agents).unwrap() {
                    assert_eq!(renderer.dispatch(intent).outcome, DispatchOutcome::Executed);
                }
            }
        }
        assert_eq!(renderer.receipts.len(), 2);
        assert!(matches!(
            renderer.intents.as_slice(),
            [
                Intent::AgentEdge {
                    slot: 0,
                    pressed: true,
                    ..
                },
                Intent::AgentEdge {
                    slot: 0,
                    pressed: false,
                    ..
                }
            ]
        ));

        let mut app = AppSnapshot::initial(&AppConfig::default());
        app.bridge.state = BridgeState::Ready;
        app.mic_state = Some(MicState::Recording);
        app.agents[0].status = AgentVisualStatus::Thinking;
        app.agents[0].selected = true;
        app.agents[0].title = Some("SECRET TITLE".into());
        app.agents[0].thread_key = Some("SECRET THREAD".into());
        app.layout.command_slots.insert(
            CommandSlot::Act06,
            crate::model::Binding::Keycap { id: "FAST".into() },
        );
        let visual = HardwareVisualState::from_app_snapshot(&app, 7);
        for write in controller.publish_visual_state(visual) {
            transport.send_from_controller(write);
        }
        let bytes = transport.receive_on_device().unwrap();
        for prohibited in [
            b"SECRET TITLE".as_slice(),
            b"SECRET THREAD".as_slice(),
            b"FAST".as_slice(),
            b"ChatGPT".as_slice(),
            b"Binding".as_slice(),
            b"account".as_slice(),
        ] {
            assert!(!bytes
                .windows(prohibited.len())
                .any(|window| window == prohibited));
        }
        device.receive(&bytes);
        assert_eq!(device.visual_states.len(), 1);
        let returned = device.visual_states[0];
        assert_eq!(returned.revision, 7);
        assert_eq!(returned.agents[0], AgentVisualState::Thinking);
        assert_eq!(returned.selected_mask, 1);
        assert_eq!(returned.mic, VisualMicState::Recording);
        assert_eq!(returned.bridge, VisualBridgeState::Ready);
        assert_eq!(
            transport.trace,
            [
                MessageType::DeviceHello,
                MessageType::GetInputSnapshot,
                MessageType::InputSnapshot,
                MessageType::InputEvent,
                MessageType::InputEvent,
                MessageType::VisualState,
            ]
        );
    }

    #[test]
    fn every_i02_ag00_visual_state_maps_to_the_closed_wire_enum() {
        for (app_status, expected) in [
            (AgentVisualStatus::Empty, AgentVisualState::Empty),
            (AgentVisualStatus::Idle, AgentVisualState::Idle),
            (AgentVisualStatus::Thinking, AgentVisualState::Thinking),
            (AgentVisualStatus::Complete, AgentVisualState::Complete),
            (AgentVisualStatus::Input, AgentVisualState::RequiresInput),
            (AgentVisualStatus::Error, AgentVisualState::Error),
        ] {
            let mut app = AppSnapshot::initial(&AppConfig::default());
            app.agents[0].status = app_status;
            let visual = HardwareVisualState::from_app_snapshot(&app, 1);
            assert_eq!(visual.wire.agents[0], expected);
            assert_eq!(visual.wire.selected_mask, 0);
        }

        let mut selected = AppSnapshot::initial(&AppConfig::default());
        selected.agents[0].status = AgentVisualStatus::Idle;
        selected.agents[0].selected = true;
        let visual = HardwareVisualState::from_app_snapshot(&selected, 2);
        assert_eq!(visual.wire.agents[0], AgentVisualState::Idle);
        assert_eq!(visual.wire.selected_mask, 0x01);
    }

    #[test]
    fn duplicate_out_of_order_gap_reboot_and_stale_generation_fail_closed() {
        let mut controller = HardwareController::default();
        let mut inputs = InputCoordinator::default();
        let connection = controller.attach(HardwareTransport::Fake, &mut inputs);
        let mut transport = FakeTransport::new(connection);
        let mut device = FakeDevice::new(10);
        establish(&mut controller, &mut inputs, &mut transport, &mut device);
        let status = controller.receive(connection, &device.status(), &mut inputs);
        assert!(status.rejection.is_none());
        assert!(status.events.is_empty());

        let press = device.agent(true);
        assert_eq!(
            controller
                .receive(connection, &press, &mut inputs)
                .events
                .len(),
            1,
            "first press"
        );
        assert_eq!(
            controller
                .receive(connection, &press, &mut inputs)
                .rejection,
            Some(SessionRejection::Duplicate)
        );

        let mut out_of_order = device.agent(false);
        out_of_order[6..10].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            controller
                .receive(connection, &out_of_order, &mut inputs)
                .rejection,
            Some(SessionRejection::OutOfOrder)
        );

        let mut gap = device.agent(false);
        gap[6..10].copy_from_slice(&6u32.to_le_bytes());
        let gap_outcome = controller.receive(connection, &gap, &mut inputs);
        assert_eq!(gap_outcome.rejection, Some(SessionRejection::Gap));
        assert_eq!(gap_outcome.events.len(), 1, "gap safely releases AG00");
        assert_eq!(gap_outcome.outbound.len(), 1);

        let mut while_desynced = device.agent(true);
        while_desynced[6..10].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(
            controller
                .receive(connection, &while_desynced, &mut inputs)
                .rejection,
            Some(SessionRejection::AwaitingSnapshot)
        );

        device.next_sequence = 8;
        let snapshot = device.snapshot(0);
        assert!(controller
            .receive(connection, &snapshot, &mut inputs)
            .rejection
            .is_none());
        let current_press = device.agent(true);
        assert_eq!(
            controller
                .receive(connection, &current_press, &mut inputs)
                .events
                .len(),
            1
        );

        let old_boot = device.boot_id;
        device.boot_id = 11;
        device.next_sequence = 1;
        let reboot = controller.receive(connection, &device.hello(), &mut inputs);
        assert!(reboot.rejection.is_none());
        assert_eq!(reboot.events.len(), 1, "reboot safely releases AG00");
        assert_eq!(reboot.outbound.len(), 1);
        assert!(controller
            .receive(connection, &device.snapshot(0), &mut inputs)
            .rejection
            .is_none());

        let stale_boot_payload = WireInputEvent {
            boot_id: old_boot,
            control_kind: WireControlKind::Agent,
            control_id: 0,
            phase: WirePhase::Pressed,
            value: 0,
        }
        .encode()
        .unwrap();
        let stale_boot = device.frame(MessageType::InputEvent, &stale_boot_payload);
        assert_eq!(
            controller
                .receive(connection, &stale_boot, &mut inputs)
                .rejection,
            Some(SessionRejection::BootMismatch)
        );

        let (reconnected, _) =
            controller.reconnect(connection, HardwareTransport::Fake, &mut inputs);
        assert_ne!(reconnected.generation, connection.generation);
        assert_eq!(
            controller
                .receive(connection, &press, &mut inputs)
                .rejection,
            Some(SessionRejection::StaleGeneration)
        );
    }

    #[test]
    fn disconnecting_one_source_does_not_release_another_sources_hold() {
        let mut controller = HardwareController::default();
        let mut inputs = InputCoordinator::default();
        let first = controller.attach(HardwareTransport::Usb, &mut inputs);
        let second = controller.attach(HardwareTransport::Ble, &mut inputs);
        let mut first_device = FakeDevice::new(1);
        let mut second_device = FakeDevice::new(2);

        for (connection, device) in [(first, &mut first_device), (second, &mut second_device)] {
            assert!(controller
                .receive(connection, &device.hello(), &mut inputs)
                .rejection
                .is_none());
            assert!(controller
                .receive(connection, &device.snapshot(0), &mut inputs)
                .rejection
                .is_none());
        }
        assert_eq!(
            controller
                .receive(first, &first_device.agent(true), &mut inputs)
                .events
                .len(),
            1
        );
        assert!(controller
            .receive(second, &second_device.agent(true), &mut inputs)
            .events
            .is_empty());
        assert!(controller.disconnect(first, &mut inputs).is_empty());
        assert_eq!(controller.disconnect(second, &mut inputs).len(), 1);
    }

    #[test]
    fn malformed_corpus_has_no_input_side_effect_and_does_not_advance_sequence() {
        let mut controller = HardwareController::default();
        let mut inputs = InputCoordinator::default();
        let connection = controller.attach(HardwareTransport::Fake, &mut inputs);
        let mut transport = FakeTransport::new(connection);
        let mut device = FakeDevice::new(0x1122_3344);
        establish(&mut controller, &mut inputs, &mut transport, &mut device);

        let wrong_direction = Frame::new(MessageType::Ping, 3, 0, &[1, 2, 3, 4])
            .unwrap()
            .encode();
        let rejected = controller.receive(connection, &wrong_direction, &mut inputs);
        assert_eq!(rejected.rejection, Some(SessionRejection::WrongDirection));
        assert!(rejected.events.is_empty());
        assert!(rejected.outbound.is_empty());

        let corpus =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fuzz/corpus");
        let mut count = 0;
        for entry in std::fs::read_dir(corpus).unwrap() {
            let path = entry.unwrap().path();
            if !path.extension().is_some_and(|extension| extension == "bin") {
                continue;
            }
            let outcome =
                controller.receive(connection, &std::fs::read(&path).unwrap(), &mut inputs);
            assert_eq!(
                outcome.rejection,
                Some(SessionRejection::Malformed),
                "{}",
                path.display()
            );
            assert!(outcome.events.is_empty(), "{}", path.display());
            assert!(outcome.outbound.is_empty(), "{}", path.display());
            count += 1;
        }
        assert_eq!(count, 95);

        let accepted = controller.receive(connection, &device.agent(true), &mut inputs);
        assert!(accepted.rejection.is_none());
        assert_eq!(accepted.events.len(), 1);
    }

    #[test]
    fn sequence_exhaustion_requires_a_new_connection_or_boot() {
        let mut session = DeviceSession::new();
        session.last_device_sequence = Some(u32::MAX);
        session.state = SessionState::Ready { boot_id: 1 };
        let payload = WireInputEvent {
            boot_id: 1,
            control_kind: WireControlKind::Agent,
            control_id: 0,
            phase: WirePhase::Pressed,
            value: 0,
        }
        .encode()
        .unwrap();
        let frame = Frame::new(MessageType::InputEvent, u32::MAX, 0, &payload).unwrap();
        let failure = session.accept(&frame).unwrap_err();
        assert_eq!(failure.rejection, SessionRejection::SequenceExhausted);
        assert!(!failure.requires_snapshot);
    }
}
