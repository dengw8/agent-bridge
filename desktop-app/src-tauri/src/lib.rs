mod adapter;
mod config;
mod control;
mod core;
mod diagnostics;
mod hardware;
mod input;
mod model;
mod passport_ble;
pub mod protocol;
mod usb_hid;

use std::{fs, future::Future, process::Command, sync::Arc, time::Duration};

use adapter::{CodexDesktopAdapter, RendererSnapshot};
use config::ConfigCommand;
use core::{AppCommand, AppCommandResult, AppCore, BridgeCommand, ConfigResult};
use hardware::ConnectionToken;
use model::{
    AppSnapshot, BridgeState, BridgeSummary, DispatchReceipt, HardwareConnectionState,
    HardwareTransport, Intent, VirtualInputEvent,
};
use passport_ble::{PassportBleEvent, PassportBleHandle};
use protocol::{Direction as ProtocolDirection, Frame, MessageType, ProtocolError};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tokio::sync::Mutex;
use usb_hid::{UsbRuntimeEvent, UsbRuntimeHandle};

struct CoreState {
    core: Arc<Mutex<AppCore>>,
    control_adapter: Arc<Mutex<CodexDesktopAdapter>>,
    input_gate: Arc<Mutex<()>>,
    bridge_gate: Arc<Mutex<()>>,
    usb_runtime: UsbRuntimeHandle,
    active_usb: Arc<Mutex<Option<ActiveUsbConnection>>>,
    passport_ble: Option<PassportBleHandle>,
    active_passport: Arc<Mutex<Option<ActivePassportConnection>>>,
    hardware_fake_renderer: bool,
}

struct TrayStatus(MenuItem<tauri::Wry>);

#[derive(Debug, Clone, Copy)]
struct ActiveUsbConnection {
    runtime_generation: u64,
    controller_token: ConnectionToken,
    device_clock_offset_ms: Option<f64>,
    host_edge_count: u64,
    receipt_count: u64,
}

#[derive(Debug, Clone, Copy)]
struct ActivePassportConnection {
    runtime_generation: u64,
    controller_token: ConnectionToken,
    published_title_mask: u8,
    published_titles: [[u8; protocol::FRAME_SIZE]; 6],
}

pub(crate) struct PollCycle {
    adapter: CodexDesktopAdapter,
    renderer: Result<RendererSnapshot, String>,
    bridge: Option<BridgeSummary>,
    control_reset_required: bool,
}

pub(crate) async fn dispatch_prepared_input<Dispatch, DispatchFuture>(
    core: &Arc<Mutex<AppCore>>,
    event: VirtualInputEvent,
    mut dispatch: Dispatch,
) -> Result<DispatchReceipt, String>
where
    Dispatch: FnMut(Intent) -> DispatchFuture,
    DispatchFuture: Future<Output = DispatchReceipt>,
{
    let intents = {
        let mut core = core.lock().await;
        core.prepare_input(event)?
    };
    let mut receipts = Vec::with_capacity(intents.len());
    for intent in intents {
        receipts.push(dispatch(intent).await);
    }
    let (receipt, releases) = {
        let mut core = core.lock().await;
        core.complete_input(receipts)
    };
    for release in releases {
        let _ = dispatch(release).await;
    }
    Ok(receipt)
}

async fn dispatch_ordered_input<Dispatch, DispatchFuture>(
    core: &Arc<Mutex<AppCore>>,
    input_gate: &Arc<Mutex<()>>,
    event: VirtualInputEvent,
    dispatch: Dispatch,
) -> Result<DispatchReceipt, String>
where
    Dispatch: FnMut(Intent) -> DispatchFuture,
    DispatchFuture: Future<Output = DispatchReceipt>,
{
    let _ordered = input_gate.lock().await;
    dispatch_prepared_input(core, event, dispatch).await
}

async fn dispatch_hardware_intents(
    core: &Arc<Mutex<AppCore>>,
    control_adapter: &Arc<Mutex<CodexDesktopAdapter>>,
    intents: Vec<Intent>,
    fake_renderer: bool,
) -> Option<AppSnapshot> {
    if intents.is_empty() {
        return None;
    }
    let mut receipts = Vec::with_capacity(intents.len());
    for intent in intents {
        receipts.push(if fake_renderer {
            DispatchReceipt {
                request_id: format!("h0-fake-{}", uuid::Uuid::new_v4()),
                intent_kind: intent.kind().into(),
                outcome: model::DispatchOutcome::Executed,
                duration_ms: 0,
                capability: Some("h0FakeRenderer".into()),
                error_code: None,
            }
        } else {
            control_adapter.lock().await.dispatch(&intent).await
        });
    }
    let (receipt, releases, snapshot) = {
        let mut core = core.lock().await;
        let (receipt, releases) = core.complete_input(receipts);
        (receipt, releases, core.snapshot())
    };
    for release in releases {
        let _ = control_adapter.lock().await.dispatch(&release).await;
    }
    let _ = receipt;
    Some(snapshot)
}

fn send_session_frames(
    runtime: &UsbRuntimeHandle,
    runtime_generation: u64,
    frames: impl IntoIterator<Item = [u8; protocol::FRAME_SIZE]>,
) {
    for bytes in frames {
        let report = Frame::decode(&bytes)
            .and_then(|frame| frame.encode_usb(ProtocolDirection::HostToDevice));
        match report {
            Ok(report) => {
                if let Err(error) = runtime.try_write(runtime_generation, report) {
                    eprintln!("Agent Bridge H0 USB output rejected: {error}");
                }
            }
            Err(_) => eprintln!("Agent Bridge H0 refused an invalid session output frame"),
        }
    }
}

async fn publish_hardware_visual(
    core: &Arc<Mutex<AppCore>>,
    runtime: &UsbRuntimeHandle,
    active_usb: &Arc<Mutex<Option<ActiveUsbConnection>>>,
) {
    let Some(active) = *active_usb.lock().await else {
        return;
    };
    let write = core
        .lock()
        .await
        .hardware_visual_write_for(active.controller_token, HardwareTransport::Usb);
    if let Some(write) = write {
        match write.usb_report() {
            Ok(report) => {
                if let Err(error) = runtime.try_write(active.runtime_generation, report) {
                    eprintln!("Agent Bridge H0 visual output rejected: {error}");
                }
            }
            Err(_) => eprintln!("Agent Bridge H0 refused an invalid visual output frame"),
        }
    }
}

fn send_passport_frames(
    runtime: &PassportBleHandle,
    runtime_generation: u64,
    frames: impl IntoIterator<Item = [u8; protocol::FRAME_SIZE]>,
) {
    for bytes in frames {
        if let Err(error) = runtime.try_write(runtime_generation, bytes) {
            eprintln!("Agent Passport BLE output rejected: {error}");
        }
    }
}

async fn publish_passport_visual(
    core: &Arc<Mutex<AppCore>>,
    runtime: &PassportBleHandle,
    active_passport: &Arc<Mutex<Option<ActivePassportConnection>>>,
) {
    let Some(active) = *active_passport.lock().await else {
        return;
    };
    let (write, title_packets) = {
        let mut core = core.lock().await;
        let titles = passport_ble::display_title_packets(&core.snapshot());
        let write = core.hardware_visual_write_for(active.controller_token, HardwareTransport::Ble);
        (write, titles)
    };
    let Some(write) = write else {
        return;
    };

    let mut connection = active_passport.lock().await;
    let Some(current) = connection.as_mut().filter(|current| {
        current.runtime_generation == active.runtime_generation
            && current.controller_token == active.controller_token
    }) else {
        return;
    };
    for (slot, packet) in title_packets.into_iter().enumerate() {
        const ALL_TITLES: u8 = 0x3f;
        let slot_mask = 1u8 << slot;
        let changed = current.published_title_mask & slot_mask == 0
            || current.published_titles[slot] != packet;
        if changed {
            match runtime.try_write_display(current.runtime_generation, packet) {
                Ok(()) => {
                    current.published_titles[slot] = packet;
                    current.published_title_mask =
                        (current.published_title_mask | slot_mask) & ALL_TITLES;
                }
                Err(error) => eprintln!("Agent Passport BLE title output rejected: {error}"),
            }
        }
    }
    if let Err(error) = runtime.try_write(current.runtime_generation, write.bytes) {
        eprintln!("Agent Passport BLE visual output rejected: {error}");
    }
}

async fn run_passport_events(
    handle: AppHandle,
    runtime: PassportBleHandle,
    mut events: tokio::sync::mpsc::Receiver<PassportBleEvent>,
    active_passport: Arc<Mutex<Option<ActivePassportConnection>>>,
) {
    while let Some(event) = events.recv().await {
        let state = handle.state::<CoreState>();
        match event {
            PassportBleEvent::Searching => {
                if active_passport.lock().await.is_none() {
                    let snapshot = state
                        .core
                        .lock()
                        .await
                        .set_hardware_connection(HardwareConnectionState::Connecting);
                    let _ = handle.emit("state_changed", snapshot);
                }
            }
            PassportBleEvent::Connected { generation } => {
                if let Some(previous) = active_passport.lock().await.take() {
                    let _ordered = state.input_gate.lock().await;
                    let intents = state
                        .core
                        .lock()
                        .await
                        .disconnect_hardware(previous.controller_token)
                        .unwrap_or_default();
                    let _ = dispatch_hardware_intents(
                        &state.core,
                        &state.control_adapter,
                        intents,
                        state.hardware_fake_renderer,
                    )
                    .await;
                }
                let token = state
                    .core
                    .lock()
                    .await
                    .attach_hardware_transport(HardwareTransport::Ble);
                *active_passport.lock().await = Some(ActivePassportConnection {
                    runtime_generation: generation,
                    controller_token: token,
                    published_title_mask: 0,
                    published_titles: [[0; protocol::FRAME_SIZE]; 6],
                });
                eprintln!("Agent Passport BLE connected generation={generation}");
            }
            PassportBleEvent::Frame {
                generation,
                bytes,
                received_monotonic_ms,
            } => {
                let token = {
                    let active = active_passport.lock().await;
                    let Some(active) = active
                        .as_ref()
                        .filter(|active| active.runtime_generation == generation)
                    else {
                        continue;
                    };
                    active.controller_token
                };
                let _ordered = state.input_gate.lock().await;
                let preparation = match state
                    .core
                    .lock()
                    .await
                    .prepare_hardware_frame(token, &bytes)
                {
                    Ok(preparation) => preparation,
                    Err(error) => {
                        eprintln!("Agent Passport BLE input failed closed: {error}");
                        continue;
                    }
                };
                send_passport_frames(&runtime, generation, preparation.outbound);
                if let Some(rejection) = preparation.rejection {
                    eprintln!(
                        "Agent Passport BLE session rejected frame at {received_monotonic_ms:.3}ms: {rejection:?}"
                    );
                }
                if let Some(snapshot) = dispatch_hardware_intents(
                    &state.core,
                    &state.control_adapter,
                    preparation.intents,
                    state.hardware_fake_renderer,
                )
                .await
                {
                    let _ = handle.emit("state_changed", snapshot);
                }
                if preparation.ready {
                    let snapshot = state
                        .core
                        .lock()
                        .await
                        .set_hardware_connection(HardwareConnectionState::Available);
                    let _ = handle.emit("state_changed", snapshot);
                }
                publish_passport_visual(&state.core, &runtime, &active_passport).await;
            }
            PassportBleEvent::Disconnected { generation } => {
                let disconnected = {
                    let mut active = active_passport.lock().await;
                    match *active {
                        Some(current) if current.runtime_generation == generation => active.take(),
                        _ => None,
                    }
                };
                if let Some(disconnected) = disconnected {
                    let _ordered = state.input_gate.lock().await;
                    let intents = state
                        .core
                        .lock()
                        .await
                        .disconnect_hardware(disconnected.controller_token)
                        .unwrap_or_default();
                    let _ = dispatch_hardware_intents(
                        &state.core,
                        &state.control_adapter,
                        intents,
                        state.hardware_fake_renderer,
                    )
                    .await;
                    let snapshot = state
                        .core
                        .lock()
                        .await
                        .set_hardware_connection(HardwareConnectionState::Disconnected);
                    let _ = handle.emit("state_changed", snapshot);
                }
                eprintln!("Agent Passport BLE disconnected generation={generation}");
            }
            PassportBleEvent::Unavailable => {
                let snapshot = state
                    .core
                    .lock()
                    .await
                    .set_hardware_connection(HardwareConnectionState::Unavailable);
                let _ = handle.emit("state_changed", snapshot);
                eprintln!("Agent Passport BLE is unavailable on this system");
            }
        }
    }
}

async fn run_usb_events(
    handle: AppHandle,
    runtime: UsbRuntimeHandle,
    mut events: tokio::sync::mpsc::Receiver<UsbRuntimeEvent>,
    active_usb: Arc<Mutex<Option<ActiveUsbConnection>>>,
) {
    while let Some(event) = events.recv().await {
        let state = handle.state::<CoreState>();
        match event {
            UsbRuntimeEvent::Connected { generation } => {
                let previous = active_usb.lock().await.take();
                if let Some(previous) = previous {
                    let _ordered = state.input_gate.lock().await;
                    let intents = state
                        .core
                        .lock()
                        .await
                        .disconnect_hardware(previous.controller_token)
                        .unwrap_or_default();
                    let _ = dispatch_hardware_intents(
                        &state.core,
                        &state.control_adapter,
                        intents,
                        state.hardware_fake_renderer,
                    )
                    .await;
                }
                let token = state.core.lock().await.attach_hardware();
                let snapshot = state
                    .core
                    .lock()
                    .await
                    .set_hardware_connection(HardwareConnectionState::Connecting);
                let _ = handle.emit("state_changed", snapshot);
                *active_usb.lock().await = Some(ActiveUsbConnection {
                    runtime_generation: generation,
                    controller_token: token,
                    device_clock_offset_ms: None,
                    host_edge_count: 0,
                    receipt_count: 0,
                });
                eprintln!("Agent Bridge H0 Vendor HID connected generation={generation}");
            }
            UsbRuntimeEvent::Report {
                generation,
                bytes,
                received_monotonic_ms,
            } => {
                let frame = Frame::decode_usb(&bytes, ProtocolDirection::DeviceToHost);
                if matches!(frame, Err(ProtocolError::UnsupportedVersion(_, _))) {
                    let snapshot = state
                        .core
                        .lock()
                        .await
                        .set_hardware_connection(HardwareConnectionState::Incompatible);
                    let _ = handle.emit("state_changed", snapshot);
                    continue;
                }
                let token = {
                    let mut active = active_usb.lock().await;
                    let Some(active) = active.as_mut() else {
                        continue;
                    };
                    if active.runtime_generation != generation {
                        continue;
                    }
                    if let Ok(frame) = &frame {
                        if frame.message_type == MessageType::DeviceHello {
                            active.device_clock_offset_ms =
                                Some(received_monotonic_ms - f64::from(frame.sender_uptime_ms));
                        } else if frame.message_type == MessageType::InputEvent {
                            active.host_edge_count += 1;
                            let transport_ms = active.device_clock_offset_ms.map(|offset| {
                                (received_monotonic_ms - offset - f64::from(frame.sender_uptime_ms))
                                    .max(0.0)
                            });
                            eprintln!(
                                "Agent Bridge H0 input generation={} sequence={} host_count={} transport_estimate_ms={}",
                                generation,
                                frame.sequence,
                                active.host_edge_count,
                                transport_ms
                                    .map(|value| format!("{value:.3}"))
                                    .unwrap_or_else(|| "unavailable".into())
                            );
                        }
                    }
                    active.controller_token
                };

                let _ordered = state.input_gate.lock().await;
                let preparation = match state
                    .core
                    .lock()
                    .await
                    .prepare_hardware_report(token, &bytes)
                {
                    Ok(preparation) => preparation,
                    Err(error) => {
                        eprintln!("Agent Bridge H0 input failed closed: {error}");
                        continue;
                    }
                };
                send_session_frames(&runtime, generation, preparation.outbound);
                if let Some(rejection) = preparation.rejection {
                    eprintln!("Agent Bridge H0 session rejected frame: {rejection:?}");
                }
                let had_intents = !preparation.intents.is_empty();
                let snapshot = dispatch_hardware_intents(
                    &state.core,
                    &state.control_adapter,
                    preparation.intents,
                    state.hardware_fake_renderer,
                )
                .await;
                if had_intents {
                    let mut active = active_usb.lock().await;
                    if let Some(active) = active
                        .as_mut()
                        .filter(|active| active.runtime_generation == generation)
                    {
                        active.receipt_count += 1;
                        eprintln!(
                            "Agent Bridge H0 receipt generation={} receipt_count={} normalized_edges={}",
                            generation,
                            active.receipt_count,
                            preparation.normalized_edges
                        );
                    }
                }
                if let Some(snapshot) = snapshot {
                    let _ = handle.emit("state_changed", snapshot);
                }
                if preparation.ready {
                    let snapshot = state
                        .core
                        .lock()
                        .await
                        .set_hardware_connection(HardwareConnectionState::Available);
                    let _ = handle.emit("state_changed", snapshot);
                }
                publish_hardware_visual(&state.core, &runtime, &active_usb).await;
            }
            UsbRuntimeEvent::Disconnected { generation } => {
                let disconnected = {
                    let mut active = active_usb.lock().await;
                    match *active {
                        Some(current) if current.runtime_generation == generation => active.take(),
                        _ => None,
                    }
                };
                if let Some(disconnected) = disconnected {
                    let _ordered = state.input_gate.lock().await;
                    let intents = state
                        .core
                        .lock()
                        .await
                        .disconnect_hardware(disconnected.controller_token)
                        .unwrap_or_default();
                    if let Some(snapshot) = dispatch_hardware_intents(
                        &state.core,
                        &state.control_adapter,
                        intents,
                        state.hardware_fake_renderer,
                    )
                    .await
                    {
                        let _ = handle.emit("state_changed", snapshot);
                    }
                    eprintln!(
                        "Agent Bridge H0 Vendor HID disconnected generation={generation}; held hardware state released"
                    );
                }
                let snapshot = state
                    .core
                    .lock()
                    .await
                    .set_hardware_connection(HardwareConnectionState::Disconnected);
                let _ = handle.emit("state_changed", snapshot);
            }
            UsbRuntimeEvent::PermissionDenied => {
                let snapshot = state
                    .core
                    .lock()
                    .await
                    .set_hardware_connection(HardwareConnectionState::PermissionDenied);
                let _ = handle.emit("state_changed", snapshot);
                eprintln!(
                    "Agent Bridge H0 USB open denied or failed; input remains disabled. Grant Input Monitoring if macOS requests it, then reconnect the device. Automatic retry remains active."
                );
            }
            UsbRuntimeEvent::Unavailable => {
                let snapshot = state
                    .core
                    .lock()
                    .await
                    .set_hardware_connection(HardwareConnectionState::Unavailable);
                let _ = handle.emit("state_changed", snapshot);
                eprintln!("Agent Bridge H0 USB adapter is unavailable on this system");
            }
        }
    }
}

pub(crate) async fn apply_poll_future<PollFuture>(
    core: Arc<Mutex<AppCore>>,
    control_adapter: Arc<Mutex<CodexDesktopAdapter>>,
    poll: PollFuture,
) -> (bool, AppSnapshot)
where
    PollFuture: Future<Output = PollCycle>,
{
    let completed = poll.await;
    {
        let mut adapter = control_adapter.lock().await;
        *adapter = completed.adapter.clone();
    }
    let mut core = core.lock().await;
    let changed = core.apply_polled_refresh(
        &completed.adapter,
        completed.renderer,
        completed.bridge,
        completed.control_reset_required,
    );
    (changed, core.snapshot())
}

fn tray_status_text(bridge: &BridgeSummary) -> String {
    let status = match bridge.state {
        BridgeState::Ready => "ChatGPT connected",
        BridgeState::Degraded => "ChatGPT connected (limited)",
        BridgeState::RestartRequired => "ChatGPT needs Bridge restart",
        BridgeState::ChatGptNotRunning => "Waiting for ChatGPT",
        BridgeState::Connecting => "Connecting to ChatGPT",
        BridgeState::AwaitingSnapshot => "Waiting for Codex state",
        BridgeState::Incompatible => "ChatGPT build is incompatible",
        BridgeState::Disconnected => "ChatGPT disconnected",
        BridgeState::Restarting => "Restarting ChatGPT",
    };
    format!("Status: {status}")
}

fn update_tray_status(handle: &AppHandle, snapshot: &AppSnapshot) {
    let status = handle.state::<TrayStatus>();
    let _ = status.0.set_text(tray_status_text(&snapshot.bridge));
}

async fn perform_bridge(handle: &AppHandle, command: BridgeCommand) -> Result<AppSnapshot, String> {
    if matches!(command, BridgeCommand::Restart { confirmed: false }) {
        return Err("Restart requires explicit confirmation.".into());
    }
    let (core, control_adapter, bridge_gate) = {
        let state = handle.state::<CoreState>();
        (
            state.core.clone(),
            state.control_adapter.clone(),
            state.bridge_gate.clone(),
        )
    };
    let _exclusive_bridge = bridge_gate.lock().await;
    let (intermediate, mut adapter) = {
        let mut core = core.lock().await;
        let intermediate = core.begin_bridge(&command);
        (intermediate, core.adapter_clone())
    };
    update_tray_status(handle, &intermediate);
    let _ = handle.emit("state_changed", &intermediate);

    let summary = match command {
        BridgeCommand::Connect | BridgeCommand::Probe => Ok(adapter.connect().await),
        BridgeCommand::Restart { confirmed } => adapter.restart_with_bridge(confirmed).await,
    };
    let summary = match summary {
        Ok(summary) => summary,
        Err(error) => {
            let failed = core.lock().await.fail_bridge(error.clone());
            *control_adapter.lock().await = adapter;
            update_tray_status(handle, &failed);
            let _ = handle.emit("state_changed", &failed);
            return Err(error);
        }
    };
    let renderer = if matches!(
        summary.state,
        BridgeState::AwaitingSnapshot | BridgeState::Ready | BridgeState::Degraded
    ) {
        adapter.refresh().await
    } else {
        Err(summary.message.clone())
    };
    let control_reset_required = adapter.take_control_reset_required();
    *control_adapter.lock().await = adapter.clone();
    let snapshot = {
        let mut core = core.lock().await;
        core.apply_polled_refresh(&adapter, renderer, Some(summary), control_reset_required);
        core.record_bridge_complete();
        core.snapshot()
    };
    update_tray_status(handle, &snapshot);
    let _ = handle.emit("state_changed", &snapshot);
    Ok(snapshot)
}

async fn configure_with_launch_item(
    core: &mut AppCore,
    command: ConfigCommand,
    mut set_launch_item: impl FnMut(bool) -> Result<(), String>,
) -> Result<ConfigResult, String> {
    let previous = core.snapshot().preferences.launch_at_login;
    let requested = match &command {
        ConfigCommand::SetPreferences {
            launch_at_login, ..
        } => *launch_at_login,
        _ => None,
    };
    let changed = requested.is_some_and(|enabled| enabled != previous);
    if let Some(enabled) = requested.filter(|_| changed) {
        set_launch_item(enabled)?;
    }
    match core.configure(command).await {
        Ok(result) => Ok(result),
        Err(error) => {
            if changed {
                set_launch_item(previous).map_err(|rollback| {
                    format!("{error}; failed to restore the login item: {rollback}")
                })?;
            }
            Err(error)
        }
    }
}

#[tauri::command]
async fn snapshot(state: State<'_, CoreState>) -> Result<AppSnapshot, String> {
    Ok(state.core.lock().await.snapshot())
}

#[tauri::command]
async fn input(
    state: State<'_, CoreState>,
    event: VirtualInputEvent,
) -> Result<DispatchReceipt, String> {
    let adapter = state.control_adapter.clone();
    dispatch_ordered_input(&state.core, &state.input_gate, event, move |intent| {
        let adapter = adapter.clone();
        async move { adapter.lock().await.dispatch(&intent).await }
    })
    .await
}

#[tauri::command]
async fn configure(
    handle: AppHandle,
    state: State<'_, CoreState>,
    command: ConfigCommand,
) -> Result<ConfigResult, String> {
    let mut core = state.core.lock().await;
    let result = configure_with_launch_item(&mut core, command, |enabled| {
        set_launch_at_login(&handle, enabled)
    })
    .await?;
    let _ = handle.emit("state_changed", &result.snapshot);
    Ok(result)
}

#[tauri::command]
async fn bridge(handle: AppHandle, command: BridgeCommand) -> Result<AppSnapshot, String> {
    perform_bridge(&handle, command).await
}

#[tauri::command]
async fn app(
    handle: AppHandle,
    state: State<'_, CoreState>,
    command: AppCommand,
) -> Result<AppCommandResult, String> {
    let (result, adapter) = {
        let mut core = state.core.lock().await;
        let result = core.app(command).await?;
        (result, core.adapter_clone())
    };
    *state.control_adapter.lock().await = adapter;
    if let Some(snapshot) = &result.snapshot {
        update_tray_status(&handle, snapshot);
        let _ = handle.emit("state_changed", snapshot);
    }
    if result.should_quit {
        handle.exit(0);
    }
    Ok(result)
}

fn show_main(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn set_launch_at_login(_handle: &AppHandle, enabled: bool) -> Result<(), String> {
    let home =
        std::env::var_os("HOME").ok_or_else(|| "Home directory is unavailable.".to_string())?;
    let directory = std::path::PathBuf::from(home).join("Library/LaunchAgents");
    let path = directory.join("com.dengwang.agent-bridge.plist");
    if !enabled {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let escaped = executable
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict>\
         <key>Label</key><string>com.dengwang.agent-bridge</string>\
         <key>ProgramArguments</key><array><string>{escaped}</string><string>--background</string></array>\
         <key>RunAtLoad</key><true/>\
         </dict></plist>\n"
    );
    let temporary = directory.join(format!(".agent-bridge-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, plist).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn confirm_with_unsent_warning(action: &str) -> bool {
    let script = format!(
        "display dialog \"Save any unsent ChatGPT Composer text first.\\n\\n{}\" \
         with title \"Agent Bridge\" buttons {{\"Cancel\", \"Continue\"}} \
         default button \"Cancel\" cancel button \"Cancel\" with icon caution",
        action.replace('"', "")
    );
    Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .is_ok_and(|status| status.success())
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<MenuItem<tauri::Wry>> {
    let status = MenuItem::with_id(
        app,
        "status",
        "Status: Starting Agent Bridge",
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(app, "show", "Show Agent Bridge", true, None::<&str>)?;
    let reconnect = MenuItem::with_id(app, "reconnect", "Reconnect", true, None::<&str>)?;
    let restart = MenuItem::with_id(
        app,
        "restart_bridge",
        "Restart ChatGPT and Connect…",
        true,
        None::<&str>,
    )?;
    let diagnostics = MenuItem::with_id(
        app,
        "diagnostics",
        "Compatibility Details",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Agent Bridge", true, None::<&str>)?;
    let quit_normal = MenuItem::with_id(
        app,
        "quit_normal",
        "Quit and Restart ChatGPT Normally…",
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &show,
            &reconnect,
            &restart,
            &diagnostics,
            &quit,
            &quit_normal,
        ],
    )?;
    TrayIconBuilder::new()
        .tooltip("Agent Bridge")
        .menu(&menu)
        .on_menu_event(|handle, event| match event.id.as_ref() {
            "show" => show_main(handle),
            "diagnostics" => {
                show_main(handle);
                let _ = handle.emit("open_diagnostics", ());
            }
            "reconnect" => {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = perform_bridge(&handle, BridgeCommand::Connect).await;
                });
            }
            "restart_bridge" => {
                if !confirm_with_unsent_warning("Restart ChatGPT with the local Bridge?") {
                    return;
                }
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ =
                        perform_bridge(&handle, BridgeCommand::Restart { confirmed: true }).await;
                });
            }
            "quit" => {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<CoreState>();
                    let _ = state.core.lock().await.app(AppCommand::Quit).await;
                    handle.exit(0);
                });
            }
            "quit_normal" => {
                if !confirm_with_unsent_warning(
                    "Quit Agent Bridge and restart ChatGPT without the debugging port?",
                ) {
                    return;
                }
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<CoreState>();
                    if state
                        .core
                        .lock()
                        .await
                        .app(AppCommand::QuitAndRestartNormal { confirmed: true })
                        .await
                        .is_ok()
                    {
                        handle.exit(0);
                    }
                });
            }
            _ => {}
        })
        .build(app)?;
    Ok(status)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let hardware_fake_renderer =
                std::env::var("AGENT_BRIDGE_H0_FAKE_RENDERER").as_deref() == Ok("1");
            let passport_ble_enabled =
                std::env::var("AGENT_BRIDGE_PASSPORT_BLE").as_deref() == Ok("1");
            let mut core = AppCore::load(&app_data).map_err(std::io::Error::other)?;
            if hardware_fake_renderer {
                core.enable_hardware_fake_renderer();
            }
            let poll_adapter = core.adapter_clone();
            let core = Arc::new(Mutex::new(core));
            let control_adapter = Arc::new(Mutex::new(poll_adapter.clone()));
            let (usb_runtime, usb_events) = usb_hid::spawn();
            let active_usb = Arc::new(Mutex::new(None));
            let (passport_ble, passport_events) = if passport_ble_enabled {
                let (runtime, events) = passport_ble::spawn();
                (Some(runtime), Some(events))
            } else {
                (None, None)
            };
            let active_passport = Arc::new(Mutex::new(None));
            app.manage(CoreState {
                core,
                control_adapter,
                input_gate: Arc::new(Mutex::new(())),
                bridge_gate: Arc::new(Mutex::new(())),
                usb_runtime: usb_runtime.clone(),
                active_usb: active_usb.clone(),
                passport_ble: passport_ble.clone(),
                active_passport: active_passport.clone(),
                hardware_fake_renderer,
            });
            if hardware_fake_renderer {
                eprintln!(
                    "Agent Bridge H0 hardware Fake Renderer is enabled; physical hardware intents will not invoke real Codex side effects"
                );
            }
            let tray_status = build_tray(app)?;
            app.manage(TrayStatus(tray_status));
            if std::env::args().any(|argument| argument == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            let usb_handle = app.handle().clone();
            tauri::async_runtime::spawn(run_usb_events(
                usb_handle,
                usb_runtime,
                usb_events,
                active_usb,
            ));
            if let (Some(runtime), Some(events)) = (passport_ble, passport_events) {
                let passport_handle = app.handle().clone();
                tauri::async_runtime::spawn(run_passport_events(
                    passport_handle,
                    runtime,
                    events,
                    active_passport,
                ));
                eprintln!(
                    "Agent Passport BLE discovery is enabled for device AgentPassport"
                );
            }

            if !hardware_fake_renderer {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut poll_adapter = poll_adapter;
                    loop {
                        let (changed, snapshot) = {
                            let state = handle.state::<CoreState>();
                            let _exclusive_bridge = state.bridge_gate.lock().await;
                            let core = state.core.clone();
                            let control_adapter = state.control_adapter.clone();
                            apply_poll_future(core, control_adapter, async {
                                let renderer = poll_adapter.refresh().await;
                                let control_reset_required =
                                    poll_adapter.take_control_reset_required();
                                let bridge = if renderer.is_err() {
                                    Some(poll_adapter.connect().await)
                                } else {
                                    None
                                };
                                PollCycle {
                                    adapter: poll_adapter.clone(),
                                    renderer,
                                    bridge,
                                    control_reset_required,
                                }
                            })
                            .await
                        };
                        if changed {
                            update_tray_status(&handle, &snapshot);
                            let _ = handle.emit("state_changed", snapshot);
                        }
                        {
                            let state = handle.state::<CoreState>();
                            publish_hardware_visual(
                                &state.core,
                                &state.usb_runtime,
                                &state.active_usb,
                            )
                            .await;
                            if let Some(passport_ble) = &state.passport_ble {
                                publish_passport_visual(
                                    &state.core,
                                    passport_ble,
                                    &state.active_passport,
                                )
                                .await;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(750)).await;
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot, input, configure, bridge, app
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agent Bridge");
}

#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;

    use crate::{
        config::{ConfigCommand, ImportPreviewItem},
        core::{AppCommand, BridgeCommand},
        model::VirtualInputEvent,
    };
    use tokio::sync::Mutex;

    #[test]
    fn public_ipc_surface_is_small_and_typed() {
        let commands = ["snapshot", "input", "configure", "bridge", "app"];
        assert_eq!(commands.len(), 5);
        assert!(!commands.contains(&"evaluate"));
        assert!(!commands.contains(&"cdpUrl"));
    }

    #[test]
    fn import_preview_items_remain_content_free() {
        let item = ImportPreviewItem {
            position: "ACT06".into(),
            current: "FAST".into(),
            imported: "TERM".into(),
            result: "TERM".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("thread"));
        assert!(!json.contains("body"));
    }

    #[test]
    fn frontend_camel_case_commands_deserialize_exactly() {
        let preferences = serde_json::from_str::<ConfigCommand>(
            r#"{"type":"setPreferences","launchAtLogin":true,"onboardingComplete":true,"themeMode":"light"}"#,
        )
        .unwrap();
        assert!(matches!(
            preferences,
            ConfigCommand::SetPreferences {
                launch_at_login: Some(true),
                onboarding_complete: Some(true),
                theme_mode: Some(crate::model::ThemePreference::Light),
            }
        ));
        assert!(serde_json::from_str::<ConfigCommand>(
            r#"{"type":"setCommand","slot":"ACT06","binding":{"type":"keycap","id":"TERM"}}"#
        )
        .is_ok());
        assert!(
            serde_json::from_str::<BridgeCommand>(r#"{"type":"restart","confirmed":true}"#).is_ok()
        );
        assert!(serde_json::from_str::<AppCommand>(
            r#"{"type":"quitAndRestartNormal","confirmed":false}"#
        )
        .is_ok());
        assert!(serde_json::from_str::<VirtualInputEvent>(
            r#"{"source":"virtualUi","control":{"type":"encoderRotate"},"phase":{"type":"rotated","steps":-1},"sequence":4,"monotonicMs":12.5}"#
        )
        .is_ok());
        for forged in [
            r#"{"source":"usb","control":{"type":"agent","slot":0},"phase":{"type":"pressed"},"sequence":1,"monotonicMs":1}"#,
            r#"{"source":{"usb":{"deviceId":"forged","generation":1}},"control":{"type":"agent","slot":0},"phase":{"type":"pressed"},"sequence":1,"monotonicMs":1}"#,
            r#"{"source":"ble","control":{"type":"agent","slot":0},"phase":{"type":"pressed"},"sequence":1,"monotonicMs":1}"#,
        ] {
            assert!(serde_json::from_str::<VirtualInputEvent>(forged).is_err());
        }
    }

    #[test]
    fn frontend_joystick_command_is_accepted_by_the_config_store() {
        let command = serde_json::from_str::<ConfigCommand>(
            r#"{"type":"setJoystick","direction":"up","binding":{"type":"command","id":"navigateBack"}}"#,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut store =
            crate::config::ConfigStore::load(directory.path().join("config.json")).unwrap();

        store.apply(command).unwrap();

        assert_eq!(
            store.current().layout.joystick[&crate::model::Direction::Up],
            crate::model::Binding::Command {
                id: "navigateBack".into()
            }
        );
    }

    #[test]
    fn tray_status_text_distinguishes_ready_from_restart_required() {
        let ready = crate::model::BridgeSummary {
            state: crate::model::BridgeState::Ready,
            message: "ready".into(),
            codex_version: Some("26.0".into()),
            fully_compatible: true,
        };
        let restart_required = crate::model::BridgeSummary {
            state: crate::model::BridgeState::RestartRequired,
            message: "restart".into(),
            codex_version: Some("26.0".into()),
            fully_compatible: false,
        };

        assert_eq!(super::tray_status_text(&ready), "Status: ChatGPT connected");
        assert_eq!(
            super::tray_status_text(&restart_required),
            "Status: ChatGPT needs Bridge restart"
        );
    }

    #[tokio::test]
    async fn launch_item_change_rolls_back_when_preferences_cannot_be_saved() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config-v1.json");
        let mut core = crate::core::AppCore::load(directory.path()).unwrap();
        std::fs::remove_file(&config_path).unwrap();
        std::fs::create_dir(&config_path).unwrap();
        let mut launch_item_changes = Vec::new();

        let result = super::configure_with_launch_item(
            &mut core,
            ConfigCommand::SetPreferences {
                launch_at_login: Some(true),
                onboarding_complete: None,
                theme_mode: None,
            },
            |enabled| {
                launch_item_changes.push(enabled);
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(launch_item_changes, vec![true, false]);
        assert!(!core.snapshot().preferences.launch_at_login);
    }

    #[tokio::test]
    async fn hardware_fake_renderer_returns_receipts_without_dispatching_adapter() {
        let directory = tempfile::tempdir().unwrap();
        let core = crate::core::AppCore::load(directory.path()).unwrap();
        let adapter = core.adapter_clone();
        let snapshot = super::dispatch_hardware_intents(
            &Arc::new(Mutex::new(core)),
            &Arc::new(Mutex::new(adapter)),
            vec![crate::model::Intent::AgentEdge {
                slot: 0,
                thread_key: None,
                pressed: true,
            }],
            true,
        )
        .await
        .unwrap();

        let receipt = snapshot.last_receipt.unwrap();
        assert_eq!(receipt.outcome, crate::model::DispatchOutcome::Executed);
        assert_eq!(receipt.capability.as_deref(), Some("h0FakeRenderer"));
        assert!(receipt.request_id.starts_with("h0-fake-"));
    }
}
