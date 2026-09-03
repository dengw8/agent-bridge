use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    model::AppSnapshot,
    protocol::{Direction, Frame, FRAME_SIZE},
};

pub(crate) const PASSPORT_DEVICE_NAME: &str = "AgentPassport";
pub(crate) const PASSPORT_SERVICE_UUID: Uuid =
    Uuid::from_u128(0x6c6f7a10_5f6c_4d22_9bce_0a6ca7296f53);
pub(crate) const PASSPORT_HOST_TO_DEVICE_UUID: Uuid =
    Uuid::from_u128(0x6c6f7a11_5f6c_4d22_9bce_0a6ca7296f53);
pub(crate) const PASSPORT_DEVICE_TO_HOST_UUID: Uuid =
    Uuid::from_u128(0x6c6f7a12_5f6c_4d22_9bce_0a6ca7296f53);
const EVENT_QUEUE_CAPACITY: usize = 32;
const COMMAND_QUEUE_CAPACITY: usize = 32;
const BLE_VALUE_CAPACITY: usize = 20;
const FRAGMENT_HEADER_SIZE: usize = 5;
const FRAGMENT_DATA_CAPACITY: usize = BLE_VALUE_CAPACITY - FRAGMENT_HEADER_SIZE;
const REASSEMBLY_TIMEOUT: Duration = Duration::from_millis(100);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(500);
const DISPLAY_PACKET_MAGIC: u8 = 0xad;
const DISPLAY_PACKET_VERSION: u8 = 1;
const DISPLAY_PACKET_AGENT_TITLE: u8 = 1;
const DISPLAY_TITLE_MAX_BYTES: usize = FRAME_SIZE - 6;

#[derive(Debug)]
pub(crate) enum PassportBleEvent {
    Searching,
    Connected {
        generation: u64,
    },
    Frame {
        generation: u64,
        bytes: [u8; FRAME_SIZE],
        received_monotonic_ms: f64,
    },
    Disconnected {
        generation: u64,
    },
    Unavailable,
}

#[derive(Debug)]
enum PassportBleCommand {
    Write {
        generation: u64,
        bytes: [u8; FRAME_SIZE],
    },
}

#[derive(Clone)]
pub(crate) struct PassportBleHandle {
    commands: mpsc::Sender<PassportBleCommand>,
}

impl PassportBleHandle {
    pub(crate) fn try_write(
        &self,
        generation: u64,
        bytes: [u8; FRAME_SIZE],
    ) -> Result<(), &'static str> {
        Frame::decode(&bytes)
            .and_then(|frame| {
                if frame.message_type.direction() == Direction::HostToDevice {
                    Ok(frame)
                } else {
                    Err(crate::protocol::ProtocolError::WrongDirection)
                }
            })
            .map_err(|_| "invalid_passport_output_frame")?;
        self.commands
            .try_send(PassportBleCommand::Write { generation, bytes })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "passport_output_queue_full",
                mpsc::error::TrySendError::Closed(_) => "passport_ble_runtime_stopped",
            })
    }

    pub(crate) fn try_write_display(
        &self,
        generation: u64,
        bytes: [u8; FRAME_SIZE],
    ) -> Result<(), &'static str> {
        if !valid_display_title_packet(&bytes) {
            return Err("invalid_passport_display_packet");
        }
        self.commands
            .try_send(PassportBleCommand::Write { generation, bytes })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "passport_output_queue_full",
                mpsc::error::TrySendError::Closed(_) => "passport_ble_runtime_stopped",
            })
    }
}

pub(crate) fn display_title_packets(snapshot: &AppSnapshot) -> [[u8; FRAME_SIZE]; 6] {
    let mut packets = [[0u8; FRAME_SIZE]; 6];
    for slot in 0..6u8 {
        packets[usize::from(slot)] = encode_display_title(slot, None);
    }
    for agent in &snapshot.agents {
        if agent.slot < 6 {
            packets[usize::from(agent.slot)] =
                encode_display_title(agent.slot, agent.title.as_deref());
        }
    }
    packets
}

fn encode_display_title(slot: u8, title: Option<&str>) -> [u8; FRAME_SIZE] {
    debug_assert!(slot < 6);
    let normalized = title
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut length = normalized.len().min(DISPLAY_TITLE_MAX_BYTES);
    while !normalized.is_char_boundary(length) {
        length -= 1;
    }

    let mut packet = [0u8; FRAME_SIZE];
    packet[0] = DISPLAY_PACKET_MAGIC;
    packet[1] = DISPLAY_PACKET_VERSION;
    packet[2] = DISPLAY_PACKET_AGENT_TITLE;
    packet[3] = slot;
    packet[4] = length as u8;
    packet[5..5 + length].copy_from_slice(&normalized.as_bytes()[..length]);
    packet
}

fn valid_display_title_packet(packet: &[u8; FRAME_SIZE]) -> bool {
    let length = usize::from(packet[4]);
    packet[0] == DISPLAY_PACKET_MAGIC
        && packet[1] == DISPLAY_PACKET_VERSION
        && packet[2] == DISPLAY_PACKET_AGENT_TITLE
        && packet[3] < 6
        && length <= DISPLAY_TITLE_MAX_BYTES
        && std::str::from_utf8(&packet[5..5 + length])
            .is_ok_and(|title| title.chars().all(|character| !character.is_control()))
        && packet[5 + length..].iter().all(|byte| *byte == 0)
}

pub(crate) fn spawn() -> (PassportBleHandle, mpsc::Receiver<PassportBleEvent>) {
    let (event_sender, event_receiver) = mpsc::channel(EVENT_QUEUE_CAPACITY);
    let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    tauri::async_runtime::spawn(run(event_sender, command_receiver));
    (
        PassportBleHandle {
            commands: command_sender,
        },
        event_receiver,
    )
}

#[derive(Debug)]
struct BleReassembler {
    transfer_id: u8,
    fragment_count: u8,
    next_index: u8,
    chunk_capacity: usize,
    length: usize,
    started_at: Option<Instant>,
    bytes: [u8; FRAME_SIZE],
}

impl Default for BleReassembler {
    fn default() -> Self {
        Self {
            transfer_id: 0,
            fragment_count: 0,
            next_index: 0,
            chunk_capacity: 0,
            length: 0,
            started_at: None,
            bytes: [0; FRAME_SIZE],
        }
    }
}

impl BleReassembler {
    fn reset(&mut self) {
        self.fragment_count = 0;
        self.next_index = 0;
        self.chunk_capacity = 0;
        self.length = 0;
        self.started_at = None;
        self.bytes.fill(0);
    }

    fn push(&mut self, value: &[u8], now: Instant) -> Option<[u8; FRAME_SIZE]> {
        if value.len() < FRAGMENT_HEADER_SIZE {
            self.reset();
            return None;
        }
        let transfer_id = value[0];
        let index = value[1];
        let count = value[2];
        let frame_length = usize::from(value[3]);
        let fragment_length = usize::from(value[4]);
        if count == 0
            || count > 16
            || frame_length != FRAME_SIZE
            || fragment_length == 0
            || fragment_length > FRAGMENT_DATA_CAPACITY
            || value.len() != FRAGMENT_HEADER_SIZE + fragment_length
        {
            self.reset();
            return None;
        }

        if index == 0 {
            self.reset();
            self.transfer_id = transfer_id;
            self.fragment_count = count;
            self.chunk_capacity = fragment_length;
            self.started_at = Some(now);
        }
        let timed_out = self
            .started_at
            .is_some_and(|started| now.duration_since(started) > REASSEMBLY_TIMEOUT);
        if timed_out
            || self.started_at.is_none()
            || transfer_id != self.transfer_id
            || count != self.fragment_count
            || index != self.next_index
            || (index + 1 < count && fragment_length != self.chunk_capacity)
        {
            self.reset();
            return None;
        }

        let expected_offset = usize::from(index) * self.chunk_capacity;
        let is_final = index + 1 == count;
        let expected_final = FRAME_SIZE.saturating_sub(expected_offset);
        if expected_offset != self.length
            || expected_offset + fragment_length > FRAME_SIZE
            || (is_final && fragment_length != expected_final)
        {
            self.reset();
            return None;
        }

        self.bytes[self.length..self.length + fragment_length]
            .copy_from_slice(&value[FRAGMENT_HEADER_SIZE..]);
        self.length += fragment_length;
        self.next_index += 1;
        if !is_final {
            return None;
        }
        let completed = (self.length == FRAME_SIZE).then_some(self.bytes);
        self.reset();
        completed
    }
}

fn fragments(frame: &[u8; FRAME_SIZE], transfer_id: u8) -> Vec<Vec<u8>> {
    let count = FRAME_SIZE.div_ceil(FRAGMENT_DATA_CAPACITY) as u8;
    (0..count)
        .map(|index| {
            let offset = usize::from(index) * FRAGMENT_DATA_CAPACITY;
            let length = (FRAME_SIZE - offset).min(FRAGMENT_DATA_CAPACITY);
            let mut value = Vec::with_capacity(FRAGMENT_HEADER_SIZE + length);
            value.extend_from_slice(&[transfer_id, index, count, FRAME_SIZE as u8, length as u8]);
            value.extend_from_slice(&frame[offset..offset + length]);
            value
        })
        .collect()
}

#[cfg(target_os = "macos")]
async fn run(
    events: mpsc::Sender<PassportBleEvent>,
    mut commands: mpsc::Receiver<PassportBleCommand>,
) {
    use btleplug::{
        api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType},
        platform::Manager,
    };
    use futures_util::StreamExt;

    let started = Instant::now();
    let manager = match Manager::new().await {
        Ok(manager) => manager,
        Err(_) => {
            let _ = events.send(PassportBleEvent::Unavailable).await;
            return;
        }
    };
    let adapter = match manager.adapters().await {
        Ok(adapters) => match adapters.into_iter().next() {
            Some(adapter) => adapter,
            None => {
                let _ = events.send(PassportBleEvent::Unavailable).await;
                return;
            }
        },
        Err(_) => {
            let _ = events.send(PassportBleEvent::Unavailable).await;
            return;
        }
    };

    let mut generation = 0u64;
    let mut transfer_id = 0u8;
    loop {
        if events.send(PassportBleEvent::Searching).await.is_err() {
            return;
        }
        let _ = adapter
            .start_scan(ScanFilter {
                services: vec![PASSPORT_SERVICE_UUID],
            })
            .await;

        let peripheral = loop {
            if commands.is_closed() {
                let _ = adapter.stop_scan().await;
                return;
            }
            let candidates = adapter.peripherals().await.unwrap_or_default();
            let mut found = None;
            for candidate in candidates {
                let properties = candidate.properties().await.ok().flatten();
                let matches = properties.as_ref().is_some_and(|properties| {
                    properties.services.contains(&PASSPORT_SERVICE_UUID)
                        || properties.local_name.as_deref() == Some(PASSPORT_DEVICE_NAME)
                });
                if matches {
                    found = Some(candidate);
                    break;
                }
            }
            if let Some(found) = found {
                break found;
            }
            tokio::time::sleep(DISCOVERY_INTERVAL).await;
        };
        let _ = adapter.stop_scan().await;
        if peripheral.connect().await.is_err() || peripheral.discover_services().await.is_err() {
            let _ = peripheral.disconnect().await;
            tokio::time::sleep(DISCOVERY_INTERVAL).await;
            continue;
        }

        let characteristics = peripheral.characteristics();
        let host_to_device = characteristics
            .iter()
            .find(|characteristic| characteristic.uuid == PASSPORT_HOST_TO_DEVICE_UUID)
            .cloned();
        let device_to_host = characteristics
            .iter()
            .find(|characteristic| characteristic.uuid == PASSPORT_DEVICE_TO_HOST_UUID)
            .cloned();
        let (Some(host_to_device), Some(device_to_host)) = (host_to_device, device_to_host) else {
            let _ = peripheral.disconnect().await;
            tokio::time::sleep(DISCOVERY_INTERVAL).await;
            continue;
        };
        let write_type = if host_to_device
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };
        let mut notifications = match peripheral.notifications().await {
            Ok(notifications) => notifications,
            Err(_) => {
                let _ = peripheral.disconnect().await;
                continue;
            }
        };
        if peripheral.subscribe(&device_to_host).await.is_err() {
            let _ = peripheral.disconnect().await;
            continue;
        }

        generation = generation.wrapping_add(1).max(1);
        if events
            .send(PassportBleEvent::Connected { generation })
            .await
            .is_err()
        {
            return;
        }
        let mut reassembler = BleReassembler::default();
        let mut health = tokio::time::interval(Duration::from_millis(750));
        let mut connected = true;
        while connected {
            tokio::select! {
                notification = notifications.next() => {
                    match notification {
                        Some(notification) if notification.uuid == PASSPORT_DEVICE_TO_HOST_UUID => {
                            if let Some(bytes) = reassembler.push(&notification.value, Instant::now()) {
                                if Frame::decode(&bytes).is_ok_and(|frame| {
                                    frame.message_type.direction() == Direction::DeviceToHost
                                }) {
                                    let event = PassportBleEvent::Frame {
                                        generation,
                                        bytes,
                                        received_monotonic_ms: started.elapsed().as_secs_f64() * 1000.0,
                                    };
                                    if events.send(event).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        Some(_) => {}
                        None => connected = false,
                    }
                }
                command = commands.recv() => {
                    let Some(PassportBleCommand::Write { generation: target, bytes }) = command else {
                        return;
                    };
                    if target != generation {
                        continue;
                    }
                    transfer_id = transfer_id.wrapping_add(1);
                    for value in fragments(&bytes, transfer_id) {
                        if peripheral.write(&host_to_device, &value, write_type).await.is_err() {
                            connected = false;
                            break;
                        }
                    }
                }
                _ = health.tick() => {
                    connected = peripheral.is_connected().await.unwrap_or(false);
                }
            }
        }
        let _ = peripheral.unsubscribe(&device_to_host).await;
        let _ = peripheral.disconnect().await;
        if events
            .send(PassportBleEvent::Disconnected { generation })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(DISCOVERY_INTERVAL).await;
    }
}

#[cfg(not(target_os = "macos"))]
async fn run(
    events: mpsc::Sender<PassportBleEvent>,
    mut commands: mpsc::Receiver<PassportBleCommand>,
) {
    let _ = events.send(PassportBleEvent::Unavailable).await;
    while commands.recv().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{AppConfig, AppSnapshot},
        protocol::MessageType,
    };

    fn host_frame() -> [u8; FRAME_SIZE] {
        Frame::new(MessageType::GetInputSnapshot, 1, 0, &[])
            .unwrap()
            .encode()
    }

    #[test]
    fn five_fragments_round_trip_one_canonical_frame() {
        let frame = host_frame();
        let values = fragments(&frame, 9);
        assert_eq!(values.len(), 5);
        assert!(values.iter().all(|value| value.len() <= BLE_VALUE_CAPACITY));
        let mut reassembler = BleReassembler::default();
        let now = Instant::now();
        let mut completed = None;
        for value in values {
            completed = reassembler.push(&value, now);
        }
        assert_eq!(completed, Some(frame));
    }

    #[test]
    fn reordered_inconsistent_and_timed_out_transfers_fail_closed() {
        let frame = host_frame();
        let values = fragments(&frame, 3);
        let now = Instant::now();

        let mut reordered = BleReassembler::default();
        assert_eq!(reordered.push(&values[1], now), None);
        assert_eq!(reordered.push(&values[0], now), None);
        assert_eq!(reordered.push(&values[2], now), None);

        let mut inconsistent = BleReassembler::default();
        assert_eq!(inconsistent.push(&values[0], now), None);
        let mut wrong_transfer = values[1].clone();
        wrong_transfer[0] = 4;
        assert_eq!(inconsistent.push(&wrong_transfer, now), None);

        let mut timed_out = BleReassembler::default();
        assert_eq!(timed_out.push(&values[0], now), None);
        assert_eq!(
            timed_out.push(
                &values[1],
                now + REASSEMBLY_TIMEOUT + Duration::from_millis(1)
            ),
            None
        );
        for value in &values[2..] {
            assert_eq!(timed_out.push(value, now), None);
        }
    }

    #[test]
    fn output_boundary_rejects_device_to_host_frames() {
        let (sender, _receiver) = mpsc::channel(1);
        let handle = PassportBleHandle { commands: sender };
        let mut payload = [0u8; 24];
        payload[0] = 1;
        payload[4..20].fill(1);
        payload[20] = 2;
        let device_frame = Frame::new(MessageType::DeviceHello, 1, 0, &payload)
            .unwrap()
            .encode();
        assert_eq!(
            handle.try_write(1, device_frame),
            Err("invalid_passport_output_frame")
        );
    }

    #[test]
    fn display_title_packet_is_utf8_bounded_and_excludes_thread_identity() {
        let mut snapshot = AppSnapshot::initial(&AppConfig::default());
        snapshot.agents[2].title =
            Some("深入了解 ai-passport 项目并设计一个非常非常长的实体分身会话名称".into());
        snapshot.agents[2].thread_key = Some("SECRET THREAD ID".into());

        let packet = display_title_packets(&snapshot)[2];
        assert!(valid_display_title_packet(&packet));
        let title_length = usize::from(packet[4]);
        let title = std::str::from_utf8(&packet[5..5 + title_length]).unwrap();
        assert!(title.starts_with("深入了解 ai-passport"));
        assert!(!packet
            .windows(b"SECRET THREAD ID".len())
            .any(|window| window == b"SECRET THREAD ID"));
    }

    #[test]
    fn display_boundary_rejects_noncanonical_packets() {
        let (sender, _receiver) = mpsc::channel(1);
        let handle = PassportBleHandle { commands: sender };
        let mut packet = encode_display_title(0, Some("session"));
        packet[62] = 1;
        assert_eq!(
            handle.try_write_display(1, packet),
            Err("invalid_passport_display_packet")
        );
    }
}
