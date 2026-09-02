use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    adapter::{capabilities_fully_supported, CodexDesktopAdapter, RendererSnapshot},
    config::{ConfigCommand, ConfigStore, ImportPreviewItem},
    control::ControlEngine,
    diagnostics::Diagnostics,
    hardware::{
        ConnectionToken, HardwareController, HardwareVisualState, SessionRejection, TransportWrite,
    },
    input::InputCoordinator,
    model::{
        now_ms, AppSnapshot, BridgeState, BridgeSummary, DispatchOutcome, DispatchReceipt,
        HardwareConnectionState, HardwareTransport, InputEvent, Intent, VirtualInputEvent,
    },
};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BridgeCommand {
    Connect,
    Probe,
    Restart { confirmed: bool },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppCommand {
    ClearDiagnostics,
    ExportDiagnostics,
    ConsumeReset { confirmed: bool },
    Quit,
    QuitAndRestartNormal { confirmed: bool },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResult {
    pub snapshot: AppSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Vec<ImportPreviewItem>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCommandResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<AppSnapshot>,
    pub should_quit: bool,
}

#[derive(Debug)]
pub struct AppCore {
    config: ConfigStore,
    adapter: CodexDesktopAdapter,
    inputs: InputCoordinator,
    hardware: HardwareController,
    hardware_visual_revision: u32,
    controls: ControlEngine,
    diagnostics: Diagnostics,
    snapshot: AppSnapshot,
    export_directory: PathBuf,
}

pub(crate) struct HardwarePreparation {
    pub(crate) intents: Vec<Intent>,
    pub(crate) outbound: Vec<[u8; crate::protocol::FRAME_SIZE]>,
    pub(crate) normalized_edges: usize,
    pub(crate) rejection: Option<SessionRejection>,
    pub(crate) ready: bool,
}

impl AppCore {
    pub fn load(app_data: &Path) -> Result<Self, String> {
        let config = ConfigStore::load(app_data.join("config-v1.json"))?;
        let snapshot = AppSnapshot::initial(config.current());
        Ok(Self {
            config,
            adapter: CodexDesktopAdapter::new(app_data.join("bridge-state.json")),
            inputs: InputCoordinator::default(),
            hardware: HardwareController::default(),
            hardware_visual_revision: 0,
            controls: ControlEngine::default(),
            diagnostics: Diagnostics::default(),
            snapshot,
            export_directory: app_data.join("diagnostics"),
        })
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.snapshot.clone()
    }

    pub(crate) fn enable_hardware_fake_renderer(&mut self) {
        self.snapshot.bridge.state = BridgeState::Degraded;
        self.snapshot.bridge.message =
            "Hardware validation mode is active; Codex actions are disabled.".into();
        self.snapshot.bridge.fully_compatible = false;
    }

    pub fn adapter_clone(&self) -> CodexDesktopAdapter {
        self.adapter.clone()
    }

    pub async fn refresh(&mut self) -> bool {
        let renderer = self.adapter.refresh().await;
        let control_reset_required = self.adapter.take_control_reset_required();
        let bridge = if renderer.is_err() {
            Some(self.adapter.connect().await)
        } else {
            None
        };
        let adapter = self.adapter.clone();
        self.apply_polled_refresh(&adapter, renderer, bridge, control_reset_required)
    }

    pub fn apply_polled_refresh(
        &mut self,
        adapter: &CodexDesktopAdapter,
        renderer: Result<RendererSnapshot, String>,
        bridge: Option<BridgeSummary>,
        control_reset_required: bool,
    ) -> bool {
        let previous = self.snapshot.clone();
        self.adapter = adapter.clone();
        if control_reset_required {
            self.inputs.clear_all();
            self.controls.clear_pressed();
        }
        match renderer {
            Ok(renderer) => {
                if let Err(error) = self.config.initialize_from_codex(
                    renderer.imported_layout.clone(),
                    self.adapter.codex_version(),
                ) {
                    self.diagnostics.record(
                        "config",
                        "failed",
                        None,
                        Some("initial_import_failed"),
                    );
                    let _ = error;
                }
                let fully_compatible = capabilities_fully_supported(self.adapter.capabilities());
                self.snapshot.bridge.state = if fully_compatible {
                    BridgeState::Ready
                } else {
                    BridgeState::Degraded
                };
                self.snapshot.bridge.message = if fully_compatible {
                    "Native Codex controls are ready.".into()
                } else {
                    "Connected with explicit capability limits.".into()
                };
                self.snapshot.bridge.fully_compatible = fully_compatible;
                self.snapshot.bridge.codex_version = self.adapter.codex_version();
                self.snapshot.capabilities = self.adapter.capabilities().to_vec();
                self.snapshot.agents = renderer.normalized_agents();
                self.snapshot.agent_source = renderer.agent_source;
                self.snapshot.active_thread = renderer.active_thread;
                self.snapshot.imported_layout = Some(renderer.imported_layout);
                self.snapshot.theme = renderer.theme;
                self.snapshot.usage = renderer.usage;
                self.snapshot.mic_state = renderer.mic_state;
                self.sync_config_fields();
            }
            Err(_) => {
                self.inputs.clear_all();
                self.controls.clear_pressed();
                let had_live_snapshot = matches!(
                    previous.bridge.state,
                    BridgeState::Ready | BridgeState::Degraded
                );
                self.snapshot.bridge = bridge.unwrap_or_else(|| BridgeSummary {
                    state: BridgeState::Disconnected,
                    message: "The ChatGPT Renderer is unavailable.".into(),
                    codex_version: self.adapter.codex_version(),
                    fully_compatible: false,
                });
                self.snapshot.capabilities = self.adapter.capabilities().to_vec();
                if had_live_snapshot {
                    for agent in &mut self.snapshot.agents {
                        agent.stale = true;
                    }
                }
                self.sync_config_fields();
            }
        }
        self.snapshot.diagnostics = self.diagnostics.recent();
        self.finish_change(previous)
    }

    pub fn begin_bridge(&mut self, command: &BridgeCommand) -> AppSnapshot {
        let previous = self.snapshot.clone();
        self.inputs.clear_all();
        self.controls.clear_pressed();
        self.snapshot.bridge.state = match command {
            BridgeCommand::Restart { .. } => BridgeState::Restarting,
            BridgeCommand::Connect | BridgeCommand::Probe => BridgeState::Connecting,
        };
        self.snapshot.bridge.message = match command {
            BridgeCommand::Restart { .. } => {
                "Restarting ChatGPT with the confirmed local Bridge.".into()
            }
            BridgeCommand::Connect | BridgeCommand::Probe => {
                "Checking the current ChatGPT Renderer and native capabilities.".into()
            }
        };
        self.snapshot.bridge.fully_compatible = false;
        for agent in &mut self.snapshot.agents {
            if agent.thread_key.is_some() {
                agent.stale = true;
            }
        }
        self.finish_change(previous);
        self.snapshot()
    }

    pub fn fail_bridge(&mut self, message: String) -> AppSnapshot {
        let previous = self.snapshot.clone();
        self.inputs.clear_all();
        self.controls.clear_pressed();
        self.snapshot.bridge.state = BridgeState::Disconnected;
        self.snapshot.bridge.message = message;
        self.snapshot.bridge.fully_compatible = false;
        self.diagnostics
            .record("bridge", "failed", None, Some("bridge_operation_failed"));
        self.snapshot.diagnostics = self.diagnostics.recent();
        self.finish_change(previous);
        self.snapshot()
    }

    pub fn record_bridge_complete(&mut self) {
        self.diagnostics.record("bridge", "complete", None, None);
        self.snapshot.diagnostics = self.diagnostics.recent();
    }

    pub fn prepare_input(&mut self, event: VirtualInputEvent) -> Result<Vec<Intent>, String> {
        if !matches!(
            self.snapshot.bridge.state,
            BridgeState::Ready | BridgeState::Degraded
        ) {
            self.controls.clear_pressed();
            self.diagnostics
                .record("dispatch", "unavailable", None, Some("bridge_unavailable"));
            self.snapshot.diagnostics = self.diagnostics.recent();
            return Err("bridge_unavailable".into());
        }
        let events = self.inputs.handle_virtual(event)?;
        self.intents_for_events(events)
    }

    pub(crate) fn attach_hardware(&mut self) -> ConnectionToken {
        self.hardware
            .attach(HardwareTransport::Usb, &mut self.inputs)
    }

    pub(crate) fn set_hardware_connection(
        &mut self,
        state: HardwareConnectionState,
    ) -> AppSnapshot {
        let previous = self.snapshot.clone();
        self.snapshot.hardware_connection = state;
        self.finish_change(previous);
        self.snapshot()
    }

    pub(crate) fn prepare_hardware_report(
        &mut self,
        connection: ConnectionToken,
        report: &[u8],
    ) -> Result<HardwarePreparation, String> {
        let outcome = self
            .hardware
            .receive_usb(connection, report, &mut self.inputs);
        let normalized_edges = outcome.events.len();
        let intents = self.intents_for_events(outcome.events)?;
        Ok(HardwarePreparation {
            intents,
            outbound: outcome.outbound,
            normalized_edges,
            rejection: outcome.rejection,
            ready: self.hardware.is_ready(connection),
        })
    }

    pub(crate) fn disconnect_hardware(
        &mut self,
        connection: ConnectionToken,
    ) -> Result<Vec<Intent>, String> {
        let events = self.hardware.disconnect(connection, &mut self.inputs);
        self.intents_for_events(events)
    }

    pub(crate) fn hardware_visual_writes(&mut self) -> Vec<TransportWrite> {
        self.hardware_visual_revision = self.hardware_visual_revision.wrapping_add(1);
        let visual =
            HardwareVisualState::from_app_snapshot(&self.snapshot, self.hardware_visual_revision);
        self.hardware.publish_visual_state(visual)
    }

    fn intents_for_events(&mut self, events: Vec<InputEvent>) -> Result<Vec<Intent>, String> {
        if !matches!(
            self.snapshot.bridge.state,
            BridgeState::Ready | BridgeState::Degraded
        ) {
            if !events.is_empty() {
                self.diagnostics.record(
                    "hardware-dispatch",
                    "unavailable",
                    None,
                    Some("bridge_unavailable"),
                );
                self.snapshot.diagnostics = self.diagnostics.recent();
            }
            return Ok(Vec::new());
        }
        let mut intents = Vec::new();
        for event in events {
            intents.extend(self.controls.handle(
                event,
                &self.config.current().layout,
                &self.snapshot.agents,
            )?);
        }
        Ok(intents)
    }

    pub fn complete_input(
        &mut self,
        receipts: Vec<DispatchReceipt>,
    ) -> (DispatchReceipt, Vec<Intent>) {
        for next in &receipts {
            self.diagnostics.record(
                "dispatch",
                match next.outcome {
                    DispatchOutcome::Executed => "executed",
                    DispatchOutcome::Unavailable => "unavailable",
                    DispatchOutcome::Unsupported => "unsupported",
                    DispatchOutcome::Failed => "failed",
                },
                Some(next.duration_ms),
                next.error_code.as_deref().map(|_| "renderer_action_failed"),
            );
        }
        let receipt = receipts
            .into_iter()
            .last()
            .unwrap_or_else(|| DispatchReceipt {
                request_id: uuid::Uuid::new_v4().to_string(),
                intent_kind: "duplicateEdge".into(),
                outcome: DispatchOutcome::Executed,
                duration_ms: 0,
                capability: None,
                error_code: None,
            });
        self.snapshot.last_receipt = Some(receipt.clone());
        self.snapshot.diagnostics = self.diagnostics.recent();
        let releases = if matches!(
            receipt.outcome,
            DispatchOutcome::Unavailable | DispatchOutcome::Failed
        ) {
            self.inputs.clear_all();
            self.controls
                .reset(&self.config.current().layout, &self.snapshot.agents)
        } else {
            Vec::new()
        };
        (receipt, releases)
    }

    pub async fn configure(&mut self, command: ConfigCommand) -> Result<ConfigResult, String> {
        let previous = self.snapshot.clone();
        let mut revision = None;
        let mut preview = None;
        match command {
            ConfigCommand::PreviewImport => {
                let imported = match self.adapter.imported_layout().cloned() {
                    Some(layout) => layout,
                    None => self.adapter.refresh().await?.imported_layout,
                };
                let result = self.config.preview_import(imported)?;
                revision = Some(result.0);
                preview = Some(result.1);
            }
            ConfigCommand::CommitImport {
                revision: requested,
            } => {
                self.config
                    .commit_import(&requested, self.adapter.codex_version())?;
            }
            command => self.config.apply(command)?,
        }
        self.sync_config_fields();
        self.diagnostics.record("config", "saved", None, None);
        self.snapshot.diagnostics = self.diagnostics.recent();
        self.finish_change(previous);
        Ok(ConfigResult {
            snapshot: self.snapshot(),
            import_revision: revision,
            preview,
        })
    }

    pub async fn app(&mut self, command: AppCommand) -> Result<AppCommandResult, String> {
        match command {
            AppCommand::ClearDiagnostics => {
                self.diagnostics.clear();
                self.snapshot.diagnostics.clear();
                Ok(result_snapshot(self.snapshot()))
            }
            AppCommand::ExportDiagnostics => {
                let path = self.diagnostics.export(&self.export_directory)?;
                Ok(AppCommandResult {
                    path: Some(path.to_string_lossy().into()),
                    snapshot: None,
                    should_quit: false,
                })
            }
            AppCommand::ConsumeReset { confirmed } => {
                if !confirmed {
                    return Err("Reset credit consumption requires a hold confirmation.".into());
                }
                let receipt = self.adapter.consume_reset_credit().await?;
                self.snapshot.last_receipt = Some(receipt);
                self.diagnostics
                    .record("usage-reset", "executed", None, None);
                let _ = self.refresh().await;
                Ok(result_snapshot(self.snapshot()))
            }
            AppCommand::Quit => {
                self.release_controls().await;
                Ok(AppCommandResult {
                    path: None,
                    snapshot: None,
                    should_quit: true,
                })
            }
            AppCommand::QuitAndRestartNormal { confirmed } => {
                self.release_controls().await;
                self.adapter.restart_normal(confirmed).await?;
                Ok(AppCommandResult {
                    path: None,
                    snapshot: None,
                    should_quit: true,
                })
            }
        }
    }

    fn sync_config_fields(&mut self) {
        self.snapshot.layout = self.config.current().layout.clone();
        self.snapshot.preferences = self.config.current().preferences.clone();
        self.snapshot.initialization_source = self.config.current().initialization.source;
    }

    fn finish_change(&mut self, previous: AppSnapshot) -> bool {
        let previous_observed = self.snapshot.observed_at_ms;
        self.snapshot.observed_at_ms = previous.observed_at_ms;
        if self.snapshot != previous {
            self.snapshot.observed_at_ms = now_ms();
            true
        } else {
            self.snapshot.observed_at_ms = previous_observed;
            false
        }
    }

    async fn release_controls(&mut self) {
        self.inputs.clear_all();
        let releases = self
            .controls
            .reset(&self.config.current().layout, &self.snapshot.agents);
        for release in releases {
            let _ = self.adapter.dispatch(&release).await;
        }
    }
}

fn result_snapshot(snapshot: AppSnapshot) -> AppCommandResult {
    AppCommandResult {
        path: None,
        snapshot: Some(snapshot),
        should_quit: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BridgeSummary, CommandSlot, ControlId, DispatchOutcome, InputPhase, ThemeMode,
        VirtualInputEvent, VirtualInputSource,
    };
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{Mutex, Notify};

    #[test]
    fn initial_snapshot_contains_no_user_content_and_six_empty_slots() {
        let directory = tempfile::tempdir().unwrap();
        let core = AppCore::load(directory.path()).unwrap();
        let snapshot = core.snapshot();
        assert_eq!(snapshot.agents.len(), 6);
        assert!(snapshot.agents.iter().all(|agent| agent.title.is_none()));
        assert_eq!(snapshot.theme, ThemeMode::Dark);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("rawCdp"));
        assert!(!serialized.contains("composerBody"));
    }

    #[test]
    fn hardware_fake_renderer_uses_a_safe_dispatchable_bridge_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut core = AppCore::load(directory.path()).unwrap();

        core.enable_hardware_fake_renderer();

        let snapshot = core.snapshot();
        assert_eq!(snapshot.bridge.state, BridgeState::Degraded);
        assert!(!snapshot.bridge.fully_compatible);
        assert!(snapshot
            .bridge
            .message
            .contains("Codex actions are disabled"));
    }

    #[test]
    fn usb_ag00_uses_control_engine_and_unplug_preserves_virtual_input() {
        use crate::protocol::{
            Direction as ProtocolDirection, Frame, MessageType, WireControlKind, WireInputEvent,
            WireInputSnapshot, WirePhase,
        };

        let directory = tempfile::tempdir().unwrap();
        let mut core = AppCore::load(directory.path()).unwrap();
        core.snapshot.bridge.state = BridgeState::Ready;
        let connection = core.attach_hardware();

        let mut hello = [0u8; 24];
        hello[0..4].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        hello[4..20].fill(0x41);
        hello[20..22].copy_from_slice(&1u16.to_le_bytes());
        hello[22] = 1;
        let hello = Frame::new(MessageType::DeviceHello, 1, 10, &hello)
            .unwrap()
            .encode_usb(ProtocolDirection::DeviceToHost)
            .unwrap();
        let hello = core.prepare_hardware_report(connection, &hello).unwrap();
        assert!(hello.intents.is_empty());
        assert_eq!(hello.outbound.len(), 1);
        assert_eq!(
            Frame::decode(&hello.outbound[0]).unwrap().message_type,
            MessageType::GetInputSnapshot
        );

        let snapshot = WireInputSnapshot {
            boot_id: 0x1122_3344,
            agent_mask: 0,
            action_mask: 0,
            joystick_mask: 0,
            encoder_pressed: false,
        }
        .encode()
        .unwrap();
        let snapshot = Frame::new(MessageType::InputSnapshot, 2, 11, &snapshot)
            .unwrap()
            .encode_usb(ProtocolDirection::DeviceToHost)
            .unwrap();
        assert!(core
            .prepare_hardware_report(connection, &snapshot)
            .unwrap()
            .intents
            .is_empty());

        let pressed = WireInputEvent {
            boot_id: 0x1122_3344,
            control_kind: WireControlKind::Agent,
            control_id: 0,
            phase: WirePhase::Pressed,
            value: 0,
        }
        .encode()
        .unwrap();
        let pressed = Frame::new(MessageType::InputEvent, 3, 20, &pressed)
            .unwrap()
            .encode_usb(ProtocolDirection::DeviceToHost)
            .unwrap();
        let press = core.prepare_hardware_report(connection, &pressed).unwrap();
        assert_eq!(press.normalized_edges, 1);
        assert!(matches!(
            press.intents.as_slice(),
            [Intent::AgentEdge {
                slot: 0,
                pressed: true,
                ..
            }]
        ));
        let fake_receipt = DispatchReceipt {
            request_id: "fake-usb-1".into(),
            intent_kind: "agentEdge".into(),
            outcome: DispatchOutcome::Executed,
            duration_ms: 0,
            capability: Some("fakeRenderer".into()),
            error_code: None,
        };
        let (receipt, _) = core.complete_input(vec![fake_receipt]);
        assert_eq!(receipt.request_id, "fake-usb-1");

        let unplug_release = core.disconnect_hardware(connection).unwrap();
        assert!(matches!(
            unplug_release.as_slice(),
            [Intent::AgentEdge {
                slot: 0,
                pressed: false,
                ..
            }]
        ));

        let virtual_press = core
            .prepare_input(VirtualInputEvent {
                source: VirtualInputSource::VirtualUi,
                control: ControlId::Agent { slot: 0 },
                phase: InputPhase::Pressed,
                sequence: 1,
                monotonic_ms: 30.0,
            })
            .unwrap();
        assert!(matches!(
            virtual_press.as_slice(),
            [Intent::AgentEdge {
                slot: 0,
                pressed: true,
                ..
            }]
        ));
    }

    #[test]
    fn ipc_command_contract_has_no_raw_evaluate_variant() {
        for raw in [
            r#"{"type":"connect"}"#,
            r#"{"type":"restart","confirmed":false}"#,
        ] {
            assert!(serde_json::from_str::<BridgeCommand>(raw).is_ok());
        }
        assert!(serde_json::from_str::<BridgeCommand>(
            r#"{"type":"evaluate","expression":"secret"}"#
        )
        .is_err());
    }

    #[tokio::test]
    async fn slow_snapshot_poll_does_not_delay_input_dispatch() {
        let directory = tempfile::tempdir().unwrap();
        let mut app_core = AppCore::load(directory.path()).unwrap();
        app_core.snapshot.bridge.state = BridgeState::Ready;
        let core = Arc::new(Mutex::new(app_core));
        let control_adapter = Arc::new(Mutex::new(CodexDesktopAdapter::new(
            directory.path().join("control-bridge-state.json"),
        )));
        let poll_started = Arc::new(Notify::new());
        let poll_signal = poll_started.clone();
        let poll_adapter =
            CodexDesktopAdapter::new(directory.path().join("poll-bridge-state.json"));
        let poll = tokio::spawn(crate::apply_poll_future(
            core.clone(),
            control_adapter,
            async move {
                poll_signal.notify_one();
                tokio::time::sleep(Duration::from_millis(250)).await;
                crate::PollCycle {
                    adapter: poll_adapter,
                    renderer: Err("delayed snapshot failure".into()),
                    bridge: Some(BridgeSummary {
                        state: BridgeState::Disconnected,
                        message: "Disconnected.".into(),
                        codex_version: None,
                        fully_compatible: false,
                    }),
                    control_reset_required: false,
                }
            },
        ));
        poll_started.notified().await;
        let started = std::time::Instant::now();

        let receipt = crate::dispatch_prepared_input(
            &core,
            VirtualInputEvent {
                source: VirtualInputSource::VirtualUi,
                control: ControlId::Command {
                    slot: CommandSlot::Act06,
                },
                phase: InputPhase::Pressed,
                sequence: 1,
                monotonic_ms: 1.0,
            },
            |_intent| async {
                DispatchReceipt {
                    request_id: "test".into(),
                    intent_kind: "executeBinding".into(),
                    outcome: DispatchOutcome::Executed,
                    duration_ms: 1,
                    capability: Some("keycap.FAST".into()),
                    error_code: None,
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(receipt.outcome, DispatchOutcome::Executed);
        assert!(started.elapsed() < Duration::from_millis(100));
        poll.await.unwrap();
    }

    #[tokio::test]
    async fn mutating_inputs_remain_ordered_while_dispatch_is_async() {
        let directory = tempfile::tempdir().unwrap();
        let mut app_core = AppCore::load(directory.path()).unwrap();
        app_core.snapshot.bridge.state = BridgeState::Ready;
        let core = Arc::new(Mutex::new(app_core));
        let input_gate = Arc::new(Mutex::new(()));
        let dispatch_order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let first_started = Arc::new(Notify::new());

        let first = {
            let core = core.clone();
            let input_gate = input_gate.clone();
            let dispatch_order = dispatch_order.clone();
            let first_started = first_started.clone();
            tokio::spawn(async move {
                crate::dispatch_ordered_input(
                    &core,
                    &input_gate,
                    VirtualInputEvent {
                        source: VirtualInputSource::VirtualUi,
                        control: ControlId::Command {
                            slot: CommandSlot::Act06,
                        },
                        phase: InputPhase::Pressed,
                        sequence: 1,
                        monotonic_ms: 1.0,
                    },
                    move |_intent| {
                        let dispatch_order = dispatch_order.clone();
                        let first_started = first_started.clone();
                        async move {
                            first_started.notify_one();
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            dispatch_order.lock().unwrap().push("first");
                            DispatchReceipt {
                                request_id: "first".into(),
                                intent_kind: "executeBinding".into(),
                                outcome: DispatchOutcome::Executed,
                                duration_ms: 100,
                                capability: Some("keycap.FAST".into()),
                                error_code: None,
                            }
                        }
                    },
                )
                .await
                .unwrap()
            })
        };
        first_started.notified().await;
        let second = {
            let core = core.clone();
            let input_gate = input_gate.clone();
            let dispatch_order = dispatch_order.clone();
            tokio::spawn(async move {
                crate::dispatch_ordered_input(
                    &core,
                    &input_gate,
                    VirtualInputEvent {
                        source: VirtualInputSource::VirtualUi,
                        control: ControlId::Command {
                            slot: CommandSlot::Act07,
                        },
                        phase: InputPhase::Pressed,
                        sequence: 2,
                        monotonic_ms: 2.0,
                    },
                    move |_intent| {
                        let dispatch_order = dispatch_order.clone();
                        async move {
                            dispatch_order.lock().unwrap().push("second");
                            DispatchReceipt {
                                request_id: "second".into(),
                                intent_kind: "executeBinding".into(),
                                outcome: DispatchOutcome::Executed,
                                duration_ms: 1,
                                capability: Some("keycap.APPR".into()),
                                error_code: None,
                            }
                        }
                    },
                )
                .await
                .unwrap()
            })
        };

        first.await.unwrap();
        second.await.unwrap();
        assert_eq!(
            dispatch_order.lock().unwrap().as_slice(),
            ["first", "second"]
        );
    }

    #[test]
    fn transient_disconnect_preserves_agents_as_last_known_but_disables_bridge() {
        let directory = tempfile::tempdir().unwrap();
        let mut core = AppCore::load(directory.path()).unwrap();
        core.snapshot.bridge.state = BridgeState::Ready;
        core.snapshot.agents[0].thread_key = Some("local:test".into());
        core.snapshot.agents[0].title = Some("Task 1".into());
        let adapter = core.adapter_clone();

        core.apply_polled_refresh(
            &adapter,
            Err("renderer unavailable".into()),
            Some(BridgeSummary {
                state: BridgeState::Disconnected,
                message: "Disconnected.".into(),
                codex_version: None,
                fully_compatible: false,
            }),
            false,
        );

        let snapshot = core.snapshot();
        assert_eq!(snapshot.bridge.state, BridgeState::Disconnected);
        assert_eq!(snapshot.agents[0].title.as_deref(), Some("Task 1"));
        assert!(snapshot.agents[0].stale);
    }

    #[test]
    fn bridge_intermediate_state_disables_inputs_before_native_work() {
        let directory = tempfile::tempdir().unwrap();
        let mut core = AppCore::load(directory.path()).unwrap();
        core.snapshot.bridge.state = BridgeState::Ready;

        let intermediate = core.begin_bridge(&BridgeCommand::Probe);

        assert_eq!(intermediate.bridge.state, BridgeState::Connecting);
        assert!(!intermediate.bridge.fully_compatible);
        assert!(core
            .prepare_input(VirtualInputEvent {
                source: VirtualInputSource::VirtualUi,
                control: ControlId::Command {
                    slot: CommandSlot::Act06,
                },
                phase: InputPhase::Pressed,
                sequence: 1,
                monotonic_ms: 1.0,
            })
            .is_err());
    }
}
