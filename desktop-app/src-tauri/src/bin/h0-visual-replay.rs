#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use agent_bridge_lib::protocol::{
        AgentVisualState, Direction, Frame, MessageType, VisualBridgeState, VisualConnectionState,
        VisualMicState, VisualProfile, VisualState,
    };
    use hidapi::HidApi;

    const VID: u16 = 0xcafe;
    const PID: u16 = 0xab01;
    const USAGE_PAGE: u16 = 0xffab;
    const USAGE: u16 = 0x0001;

    let api = HidApi::new()?;
    let info = api
        .device_list()
        .find(|info| {
            info.vendor_id() == VID
                && info.product_id() == PID
                && info.usage_page() == USAGE_PAGE
                && info.usage() == USAGE
        })
        .ok_or("the exact H0 Vendor HID collection is not enumerated")?;
    let device = info.open_device(&api)?;
    let hello = read_message(&device, MessageType::DeviceHello, Duration::from_secs(5))?;
    println!(
        "H0_VISUAL_REPLAY device_hello_sequence={} sender_uptime_ms={}",
        hello.sequence, hello.sender_uptime_ms
    );

    let snapshot_request = Frame::new(MessageType::GetInputSnapshot, 1, 0, &[])?
        .encode_usb(Direction::HostToDevice)?;
    require_full_write(&device, &snapshot_request)?;
    let _ = read_message(&device, MessageType::InputSnapshot, Duration::from_secs(2))?;

    for (index, (label, agent, selected)) in [
        ("Empty", AgentVisualState::Empty, false),
        ("Idle", AgentVisualState::Idle, false),
        ("Thinking", AgentVisualState::Thinking, false),
        ("Complete", AgentVisualState::Complete, false),
        ("Input", AgentVisualState::RequiresInput, false),
        ("Error", AgentVisualState::Error, false),
        ("Selected", AgentVisualState::Idle, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut agents = [AgentVisualState::Empty; 6];
        agents[0] = agent;
        let visual = VisualState {
            revision: (index + 1) as u32,
            agents,
            selected_mask: u8::from(selected),
            mic: VisualMicState::Idle,
            bridge: VisualBridgeState::Ready,
            connection: VisualConnectionState::Ready,
            profile: VisualProfile::Usb,
            battery_percent: 255,
            power_flags: 0x02,
            brightness_percent: 50,
            auto_dim_seconds: 180,
        };
        let payload = visual.encode()?;
        let report = Frame::new(MessageType::VisualState, (index + 2) as u32, 0, &payload)?
            .encode_usb(Direction::HostToDevice)?;
        require_full_write(&device, &report)?;
        let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        println!("H0_VISUAL_REPLAY state={label} sent_unix_ms={unix_ms}");
        thread::sleep(Duration::from_secs(2));
    }
    println!(
        "H0_VISUAL_REPLAY complete; video/manual observation is still required for physical latency"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_message(
    device: &hidapi::HidDevice,
    expected: agent_bridge_lib::protocol::MessageType,
    timeout: std::time::Duration,
) -> Result<agent_bridge_lib::protocol::Frame, Box<dyn std::error::Error>> {
    use std::time::Instant;

    use agent_bridge_lib::protocol::{Direction, Frame, USB_REPORT_SIZE};

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let mut report = [0u8; USB_REPORT_SIZE];
        match device.read_timeout(&mut report, 100)? {
            0 => continue,
            USB_REPORT_SIZE => {
                let frame = Frame::decode_usb(&report, Direction::DeviceToHost)?;
                if frame.message_type == expected {
                    return Ok(frame);
                }
            }
            _ => return Err("H0 returned a non-canonical HID report length".into()),
        }
    }
    Err(format!("timed out waiting for {expected:?}").into())
}

#[cfg(target_os = "macos")]
fn require_full_write(
    device: &hidapi::HidDevice,
    report: &[u8; agent_bridge_lib::protocol::USB_REPORT_SIZE],
) -> Result<(), Box<dyn std::error::Error>> {
    if device.write(report)? != report.len() {
        return Err("H0 accepted a partial output report".into());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("H0 visual replay is supported only on macOS.");
    std::process::exit(2);
}
