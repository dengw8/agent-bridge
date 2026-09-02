use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
    time::{Duration, Instant},
};

use tokio::sync::mpsc as tokio_mpsc;

use crate::protocol::{
    Direction, Frame, USB_INPUT_REPORT_ID, USB_OUTPUT_REPORT_ID, USB_REPORT_SIZE,
};

pub(crate) const H0_USB_VENDOR_ID: u16 = 0xcafe;
pub(crate) const H0_USB_PRODUCT_ID: u16 = 0xab01;
pub(crate) const H0_USB_USAGE_PAGE: u16 = 0xffab;
pub(crate) const H0_USB_USAGE: u16 = 0x0001;

const EVENT_QUEUE_CAPACITY: usize = 32;
const COMMAND_QUEUE_CAPACITY: usize = 32;
const ENUMERATION_INTERVAL: Duration = Duration::from_millis(500);
const READ_TIMEOUT_MS: i32 = 20;

#[derive(Debug)]
pub(crate) enum UsbRuntimeEvent {
    Connected {
        generation: u64,
    },
    Report {
        generation: u64,
        bytes: [u8; USB_REPORT_SIZE],
        received_monotonic_ms: f64,
    },
    Disconnected {
        generation: u64,
    },
    PermissionDenied,
    Unavailable,
}

#[derive(Debug)]
enum UsbRuntimeCommand {
    Write {
        generation: u64,
        bytes: [u8; USB_REPORT_SIZE],
    },
}

#[derive(Clone)]
pub(crate) struct UsbRuntimeHandle {
    commands: SyncSender<UsbRuntimeCommand>,
}

impl UsbRuntimeHandle {
    pub(crate) fn try_write(
        &self,
        generation: u64,
        bytes: [u8; USB_REPORT_SIZE],
    ) -> Result<(), &'static str> {
        validate_output_report(&bytes)?;
        self.commands
            .try_send(UsbRuntimeCommand::Write { generation, bytes })
            .map_err(|error| match error {
                TrySendError::Full(_) => "usb_output_queue_full",
                TrySendError::Disconnected(_) => "usb_runtime_stopped",
            })
    }
}

pub(crate) fn spawn() -> (UsbRuntimeHandle, tokio_mpsc::Receiver<UsbRuntimeEvent>) {
    let (event_sender, event_receiver) = tokio_mpsc::channel(EVENT_QUEUE_CAPACITY);
    let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
    thread::Builder::new()
        .name("agent-bridge-h0-usb".into())
        .spawn(move || run(event_sender, command_receiver))
        .expect("failed to start the bounded H0 USB runtime");
    (
        UsbRuntimeHandle {
            commands: command_sender,
        },
        event_receiver,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HidIdentity {
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
}

fn matches_h0(identity: HidIdentity) -> bool {
    identity
        == (HidIdentity {
            vendor_id: H0_USB_VENDOR_ID,
            product_id: H0_USB_PRODUCT_ID,
            usage_page: H0_USB_USAGE_PAGE,
            usage: H0_USB_USAGE,
        })
}

fn validate_input_report(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() != USB_REPORT_SIZE {
        return Err("invalid_usb_input_length");
    }
    if bytes[0] != USB_INPUT_REPORT_ID {
        return Err("invalid_usb_input_report_id");
    }
    Ok(())
}

fn validate_output_report(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() != USB_REPORT_SIZE {
        return Err("invalid_usb_output_length");
    }
    if bytes[0] != USB_OUTPUT_REPORT_ID {
        return Err("invalid_usb_output_report_id");
    }
    Frame::decode_usb(bytes, Direction::HostToDevice)
        .map(|_| ())
        .map_err(|_| "invalid_usb_output_frame")
}

#[cfg(target_os = "macos")]
fn run(events: tokio_mpsc::Sender<UsbRuntimeEvent>, commands: Receiver<UsbRuntimeCommand>) {
    use hidapi::{HidApi, HidDevice};

    let started = Instant::now();
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(_) => {
            let _ = events.blocking_send(UsbRuntimeEvent::Unavailable);
            return;
        }
    };
    let mut device: Option<HidDevice> = None;
    let mut generation = 0u64;
    let mut last_enumeration = Instant::now() - ENUMERATION_INTERVAL;
    let mut open_failure_reported = false;
    let mut candidate_was_present = false;

    loop {
        match commands.try_recv() {
            Err(TryRecvError::Disconnected) => return,
            Ok(UsbRuntimeCommand::Write {
                generation: target,
                bytes,
            }) if target == generation => {
                if device
                    .as_ref()
                    .is_some_and(|active| active.write(&bytes).ok() != Some(bytes.len()))
                {
                    disconnect(&events, &mut device, generation);
                }
            }
            Ok(UsbRuntimeCommand::Write { .. }) | Err(TryRecvError::Empty) => {}
        }

        if device.is_none() {
            if last_enumeration.elapsed() < ENUMERATION_INTERVAL {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            last_enumeration = Instant::now();
            if api.refresh_devices().is_err() {
                continue;
            }
            let candidate = api
                .device_list()
                .find(|info| {
                    matches_h0(HidIdentity {
                        vendor_id: info.vendor_id(),
                        product_id: info.product_id(),
                        usage_page: info.usage_page(),
                        usage: info.usage(),
                    })
                })
                .cloned();
            let Some(candidate) = candidate else {
                if candidate_was_present {
                    let _ = events.blocking_send(UsbRuntimeEvent::Disconnected { generation });
                }
                candidate_was_present = false;
                open_failure_reported = false;
                continue;
            };
            candidate_was_present = true;
            match candidate.open_device(&api) {
                Ok(opened) => {
                    generation = generation.wrapping_add(1).max(1);
                    device = Some(opened);
                    open_failure_reported = false;
                    if events
                        .blocking_send(UsbRuntimeEvent::Connected { generation })
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => {
                    if !open_failure_reported {
                        let _ = events.blocking_send(UsbRuntimeEvent::PermissionDenied);
                        open_failure_reported = true;
                    }
                }
            }
            continue;
        }

        let mut report = [0u8; USB_REPORT_SIZE];
        match device
            .as_ref()
            .expect("device presence checked")
            .read_timeout(&mut report, READ_TIMEOUT_MS)
        {
            Ok(0) => {}
            Ok(length) if length == USB_REPORT_SIZE && validate_input_report(&report).is_ok() => {
                if events
                    .blocking_send(UsbRuntimeEvent::Report {
                        generation,
                        bytes: report,
                        received_monotonic_ms: started.elapsed().as_secs_f64() * 1000.0,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(_) => {
                // A malformed/wrong-ID report is rejected before the ABP session.
            }
            Err(_) => disconnect(&events, &mut device, generation),
        }
    }
}

#[cfg(target_os = "macos")]
fn disconnect(
    events: &tokio_mpsc::Sender<UsbRuntimeEvent>,
    device: &mut Option<hidapi::HidDevice>,
    generation: u64,
) {
    *device = None;
    let _ = events.blocking_send(UsbRuntimeEvent::Disconnected { generation });
}

#[cfg(not(target_os = "macos"))]
fn run(events: tokio_mpsc::Sender<UsbRuntimeEvent>, commands: Receiver<UsbRuntimeCommand>) {
    let _ = events.blocking_send(UsbRuntimeEvent::Unavailable);
    while commands.recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{MessageType, FRAME_SIZE};

    #[test]
    fn h0_matching_requires_all_four_descriptor_fields() {
        let expected = HidIdentity {
            vendor_id: H0_USB_VENDOR_ID,
            product_id: H0_USB_PRODUCT_ID,
            usage_page: H0_USB_USAGE_PAGE,
            usage: H0_USB_USAGE,
        };
        assert!(matches_h0(expected));
        for different in [
            HidIdentity {
                vendor_id: 0x303a,
                ..expected
            },
            HidIdentity {
                product_id: 0x0001,
                ..expected
            },
            HidIdentity {
                usage_page: 0x0001,
                ..expected
            },
            HidIdentity {
                usage: 0x0002,
                ..expected
            },
        ] {
            assert!(!matches_h0(different));
        }
    }

    #[test]
    fn transport_boundary_rejects_wrong_id_direction_and_length() {
        let input = Frame::new(MessageType::DeviceHello, 1, 10, &hello_payload())
            .unwrap()
            .encode_usb(Direction::DeviceToHost)
            .unwrap();
        assert!(validate_input_report(&input).is_ok());
        assert_eq!(
            validate_input_report(&input[..USB_REPORT_SIZE - 1]),
            Err("invalid_usb_input_length")
        );

        let mut wrong_id = input;
        wrong_id[0] = USB_OUTPUT_REPORT_ID;
        assert_eq!(
            validate_input_report(&wrong_id),
            Err("invalid_usb_input_report_id")
        );

        let output = Frame::new(MessageType::GetInputSnapshot, 1, 0, &[])
            .unwrap()
            .encode_usb(Direction::HostToDevice)
            .unwrap();
        assert!(validate_output_report(&output).is_ok());
        assert_eq!(input.len(), FRAME_SIZE + 1);
        assert!(validate_output_report(&input).is_err());
    }

    fn hello_payload() -> [u8; 24] {
        let mut payload = [0u8; 24];
        payload[0..4].copy_from_slice(&1u32.to_le_bytes());
        payload[4..20].fill(0x11);
        payload[20..22].copy_from_slice(&1u16.to_le_bytes());
        payload[22] = 1;
        payload
    }
}
