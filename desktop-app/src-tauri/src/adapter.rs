use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;
use uuid::Uuid;

use crate::model::{
    visual_status, ActiveThread, AgentSlot, AgentSource, Binding, BridgeState, BridgeSummary,
    Capability, CapabilityStatus, DispatchOutcome, DispatchReceipt, Intent, Layout, MicState,
    ThemeMode, UsageSnapshot, OFFICIAL_KEYCAP_IDS,
};

const SNAPSHOT_SCRIPT: &str = include_str!("../resources/renderer/snapshot.js");
const PROBE_SCRIPT: &str = include_str!("../resources/renderer/probe.js");
const BINDING_SCRIPT: &str = include_str!("../resources/renderer/binding-driver.js");
const ACTION_SCRIPT: &str = include_str!("../resources/renderer/action.js");
const CHATGPT_BUNDLE_ID: &str = "com.openai.codex";
const REQUIRED_BASE_CAPABILITIES: [&str; 8] = [
    "snapshot",
    "agent-slots",
    "native-binding-override",
    "mic-phase",
    "mic-state",
    "joystick",
    "encoder",
    "layout-import",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RendererAgent {
    slot: u8,
    thread_key: Option<String>,
    title: Option<String>,
    native_status: String,
    selected: bool,
    context_used_percent: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererSnapshot {
    agents: Vec<RendererAgent>,
    pub agent_source: AgentSource,
    pub active_thread: Option<ActiveThread>,
    pub imported_layout: Layout,
    pub theme: ThemeMode,
    pub usage: Option<UsageSnapshot>,
    pub mic_state: Option<MicState>,
}

impl RendererSnapshot {
    pub fn normalized_agents(&self) -> Vec<AgentSlot> {
        self.agents
            .iter()
            .map(|agent| AgentSlot {
                slot: agent.slot,
                thread_key: agent.thread_key.clone(),
                title: agent.title.clone(),
                status: visual_status(&agent.native_status),
                native_status: agent.native_status.clone(),
                selected: agent.selected,
                context_used_percent: agent.context_used_percent,
                stale: false,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ProbeKeycap {
    id: String,
    available: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RendererProbe {
    keycaps: Vec<ProbeKeycap>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BindingProbe {
    native_mic_state_machine: bool,
    native_encoder_gesture: bool,
    native_joystick_handler: bool,
    persisted_layout_isolated: bool,
    native_mic_state_signal: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ActionResult {
    outcome: DispatchOutcome,
    capability: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DebugTarget {
    id: String,
    #[serde(rename = "type")]
    target_type: String,
    url: String,
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ChatGptProcess {
    pid: u32,
    command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeStateFile {
    port: u16,
    generation: String,
    started_at: String,
}

#[derive(Debug, Clone)]
pub struct CodexDesktopAdapter {
    state_file: PathBuf,
    port: Option<u16>,
    capabilities: Vec<Capability>,
    version: Option<String>,
    last_renderer_snapshot: Option<RendererSnapshot>,
    target_generation: Option<String>,
    capability_generation: Option<String>,
    control_reset_required: bool,
}

impl CodexDesktopAdapter {
    pub fn new(state_file: PathBuf) -> Self {
        Self {
            state_file,
            port: None,
            capabilities: Vec::new(),
            version: None,
            last_renderer_snapshot: None,
            target_generation: None,
            capability_generation: None,
            control_reset_required: false,
        }
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn imported_layout(&self) -> Option<&Layout> {
        self.last_renderer_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.imported_layout)
    }

    pub fn codex_version(&self) -> Option<String> {
        self.version.clone()
    }

    pub fn take_control_reset_required(&mut self) -> bool {
        std::mem::take(&mut self.control_reset_required)
    }

    pub async fn connect(&mut self) -> BridgeSummary {
        self.invalidate_capabilities();
        self.last_renderer_snapshot = None;
        self.target_generation = None;
        let processes = match running_chatgpt_processes() {
            Ok(processes) => processes,
            Err(_) => {
                return summary(
                    BridgeState::Disconnected,
                    "Unable to inspect the ChatGPT process.",
                    self.version.clone(),
                    false,
                )
            }
        };
        if processes.is_empty() {
            self.port = None;
            return summary(
                BridgeState::ChatGptNotRunning,
                "Open ChatGPT Desktop when you are ready; Agent Bridge will not start it automatically.",
                self.version.clone(),
                false,
            );
        }
        self.version = discover_chatgpt_bundle()
            .ok()
            .and_then(|path| bundle_value(&path, "CFBundleShortVersionString").ok());
        let port = port_from_processes(&processes);
        let Some(port) = port else {
            self.port = None;
            return summary(
                BridgeState::RestartRequired,
                "Save any unsent Composer text, then confirm a restart to enable the local loopback Bridge.",
                self.version.clone(),
                false,
            );
        };
        if !is_loopback_debug_endpoint(port).await {
            self.port = None;
            return summary(
                BridgeState::RestartRequired,
                "ChatGPT is running without a healthy local Bridge. Save unsent Composer text before restarting.",
                self.version.clone(),
                false,
            );
        }
        self.port = Some(port);
        if self.read_snapshot().await.is_err() {
            return summary(
                BridgeState::Incompatible,
                "The current ChatGPT Renderer could not produce a native status snapshot.",
                self.version.clone(),
                false,
            );
        }
        match self.probe().await {
            Ok(capabilities) => {
                self.capabilities = capabilities;
                self.capability_generation = self.target_generation.clone();
                let fully_compatible = capabilities_fully_supported(&self.capabilities);
                summary(
                    if fully_compatible {
                        BridgeState::AwaitingSnapshot
                    } else {
                        BridgeState::Degraded
                    },
                    if fully_compatible {
                        "Connected to ChatGPT's main Codex Renderer."
                    } else {
                        "Connected, but one or more native capabilities changed."
                    },
                    self.version.clone(),
                    fully_compatible,
                )
            }
            Err(_) => summary(
                BridgeState::Incompatible,
                "The current ChatGPT Renderer could not satisfy the native capability probe.",
                self.version.clone(),
                false,
            ),
        }
    }

    pub async fn refresh(&mut self) -> Result<RendererSnapshot, String> {
        if self.port.is_none() {
            let bridge = self.connect().await;
            if !matches!(
                bridge.state,
                BridgeState::AwaitingSnapshot | BridgeState::Ready | BridgeState::Degraded
            ) {
                return Err(bridge.message);
            }
        }
        let snapshot = self.read_snapshot().await?;
        if self.capability_generation != self.target_generation {
            let capabilities = self.probe().await?;
            self.capabilities = capabilities;
            self.capability_generation = self.target_generation.clone();
        }
        Ok(snapshot)
    }

    async fn read_snapshot(&mut self) -> Result<RendererSnapshot, String> {
        let value = self.evaluate_snapshot(SNAPSHOT_SCRIPT).await?;
        let snapshot: RendererSnapshot = serde_json::from_value(value)
            .map_err(|error| format!("Invalid Renderer snapshot: {error}"))?;
        snapshot.imported_layout.validate_importable()?;
        if snapshot.agents.len() != 6 {
            return Err("Renderer snapshot did not contain six Agent slots.".into());
        }
        self.last_renderer_snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub async fn dispatch(&mut self, intent: &Intent) -> DispatchReceipt {
        let started = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let result = self.dispatch_inner(intent).await;
        match result {
            Ok(action) => DispatchReceipt {
                request_id,
                intent_kind: intent.kind().into(),
                outcome: action.outcome,
                duration_ms: started.elapsed().as_millis() as u64,
                capability: action.capability,
                error_code: None,
            },
            Err(code) => DispatchReceipt {
                request_id,
                intent_kind: intent.kind().into(),
                outcome: if self.port.is_none() {
                    DispatchOutcome::Unavailable
                } else {
                    DispatchOutcome::Failed
                },
                duration_ms: started.elapsed().as_millis() as u64,
                capability: None,
                error_code: Some(code),
            },
        }
    }

    async fn dispatch_inner(&mut self, intent: &Intent) -> Result<ActionResult, String> {
        if self.port.is_none() {
            return Err("bridge_unavailable".into());
        }
        let request = match intent {
            Intent::ExecuteBinding {
                slot: Some(slot),
                binding,
                pressed,
            } => json!({
                "kind": "binding",
                "slot": slot,
                "binding": binding,
                "pressed": pressed,
            }),
            Intent::ExecuteBinding {
                slot: None,
                binding: Binding::Keycap { id },
                pressed,
            } => {
                if !pressed {
                    return Ok(executed(format!("keycap.{id}")));
                }
                return self
                    .evaluate_action(json!({ "kind": "keycap", "id": id }))
                    .await;
            }
            Intent::ExecuteBinding { .. } => return Err("invalid_standalone_binding".into()),
            Intent::JoystickEdge {
                direction,
                active,
                binding,
            } => json!({
                "kind": "joystick",
                "direction": direction,
                "binding": binding,
                "active": active,
            }),
            Intent::EncoderEdge { mode, pressed } => json!({
                "kind": "encoder",
                "mode": mode,
                "phase": if *pressed { "pressed" } else { "released" },
            }),
            Intent::EncoderRotate { mode, steps } => json!({
                "kind": "encoder",
                "mode": mode,
                "phase": if *steps > 0 { "clockwise" } else { "counterclockwise" },
                "steps": steps.unsigned_abs(),
            }),
            Intent::AgentEdge {
                slot,
                thread_key,
                pressed,
            } => {
                return self
                    .dispatch_agent(*slot, thread_key.as_deref(), *pressed)
                    .await;
            }
        };
        let expression = inject_request(BINDING_SCRIPT, &request)?;
        let value = self.evaluate(&expression).await?;
        serde_json::from_value(value).map_err(|_| "invalid_binding_receipt".into())
    }

    async fn dispatch_agent(
        &mut self,
        requested_slot: u8,
        expected_thread: Option<&str>,
        pressed: bool,
    ) -> Result<ActionResult, String> {
        let snapshot = if pressed {
            Some(self.refresh().await?)
        } else {
            self.last_renderer_snapshot.clone()
        };
        let current_slot = expected_thread.and_then(|thread| {
            snapshot
                .as_ref()?
                .agents
                .iter()
                .find(|agent| same_thread_identity(agent.thread_key.as_deref(), Some(thread)))
                .map(|agent| agent.slot)
        });
        if pressed && expected_thread.is_some() && current_slot.is_none() {
            return self
                .evaluate_action(json!({
                    "kind": "openThread",
                    "threadKey": expected_thread,
                }))
                .await;
        }
        self.evaluate_action(json!({
            "kind": "agent",
            "slot": current_slot.unwrap_or(requested_slot),
            "threadKey": expected_thread,
            "pressed": pressed,
        }))
        .await
    }

    pub async fn consume_reset_credit(&mut self) -> Result<DispatchReceipt, String> {
        let started = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let action = self
            .evaluate_action(json!({
                "kind": "consumeReset",
                "requestId": request_id,
            }))
            .await?;
        Ok(DispatchReceipt {
            request_id,
            intent_kind: "consumeReset".into(),
            outcome: action.outcome,
            duration_ms: started.elapsed().as_millis() as u64,
            capability: action.capability,
            error_code: None,
        })
    }

    async fn evaluate_action(&mut self, request: Value) -> Result<ActionResult, String> {
        let expression = inject_request(ACTION_SCRIPT, &request)?;
        let value = self.evaluate(&expression).await?;
        serde_json::from_value(value).map_err(|_| "invalid_action_receipt".into())
    }

    async fn probe(&mut self) -> Result<Vec<Capability>, String> {
        self.begin_probe();
        let renderer: RendererProbe = serde_json::from_value(self.evaluate(PROBE_SCRIPT).await?)
            .map_err(|error| error.to_string())?;
        let binding_expression = inject_request(BINDING_SCRIPT, &json!({ "kind": "probe" }))?;
        let binding: BindingProbe =
            serde_json::from_value(self.evaluate(&binding_expression).await?)
                .map_err(|error| error.to_string())?;
        let snapshot = self.last_renderer_snapshot.as_ref().ok_or_else(|| {
            "A validated Renderer snapshot is required before probing.".to_string()
        })?;
        let agent_slots = snapshot.agents.len() == 6
            && snapshot
                .agents
                .iter()
                .enumerate()
                .all(|(index, agent)| agent.slot == index as u8);
        let native_binding = binding.native_mic_state_machine
            && binding.native_encoder_gesture
            && binding.native_joystick_handler
            && binding.persisted_layout_isolated;
        let mut capabilities = vec![
            capability("snapshot", "Native status snapshot", true),
            capability("agent-slots", "Six native Agent slots", agent_slots),
            capability(
                "native-binding-override",
                "Independent native Layout",
                native_binding,
            ),
            capability(
                "mic-phase",
                "Native push-to-talk phases",
                binding.native_mic_state_machine,
            ),
            capability(
                "mic-state",
                "Native microphone state",
                binding.native_mic_state_signal,
            ),
            capability(
                "joystick",
                "Native analog stick handler",
                binding.native_joystick_handler,
            ),
            capability(
                "encoder",
                "Native encoder gestures",
                binding.native_encoder_gesture,
            ),
            capability(
                "layout-import",
                "Codex Micro Layout import",
                layout_import_supported(&snapshot.imported_layout),
            ),
            capability("usage", "Usage and reset credits", snapshot.usage.is_some()),
        ];
        for id in OFFICIAL_KEYCAP_IDS {
            let supported = renderer
                .keycaps
                .iter()
                .any(|keycap| keycap.id == id && keycap.available);
            capabilities.push(capability(
                &format!("keycap.{id}"),
                &format!("Keycap {id}"),
                supported,
            ));
        }
        Ok(capabilities)
    }

    async fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_with_target_policy(expression, false).await
    }

    async fn evaluate_snapshot(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_with_target_policy(expression, true).await
    }

    async fn evaluate_with_target_policy(
        &mut self,
        expression: &str,
        allow_target_change: bool,
    ) -> Result<Value, String> {
        let port = self.port.ok_or_else(|| "bridge_unavailable".to_string())?;
        let target = match main_debug_target(port).await {
            Ok(target) => target,
            Err(error) => {
                self.port = None;
                self.invalidate_capabilities();
                return Err(error);
            }
        };
        if let Err(error) = self.observe_target_generation(&target.id, allow_target_change) {
            self.port = None;
            return Err(error);
        }
        match cdp_evaluate(port, &target, expression).await {
            Ok(value) => Ok(value),
            Err(error) => {
                self.port = None;
                self.invalidate_capabilities();
                Err(error)
            }
        }
    }

    fn observe_target_generation(
        &mut self,
        generation: &str,
        allow_target_change: bool,
    ) -> Result<(), String> {
        let changed = self
            .target_generation
            .as_deref()
            .is_some_and(|current| current != generation);
        if changed {
            self.invalidate_capabilities();
            self.last_renderer_snapshot = None;
            self.control_reset_required = true;
        }
        self.target_generation = Some(generation.to_string());
        if changed && !allow_target_change {
            return Err("renderer_generation_changed".into());
        }
        Ok(())
    }

    fn invalidate_capabilities(&mut self) {
        self.capabilities.clear();
        self.capability_generation = None;
    }

    fn begin_probe(&mut self) {
        self.invalidate_capabilities();
    }

    pub async fn restart_with_bridge(&mut self, confirmed: bool) -> Result<BridgeSummary, String> {
        if !confirmed {
            return Err("Restart requires explicit confirmation.".into());
        }
        let processes = running_chatgpt_processes()?;
        if processes.is_empty() {
            return Ok(summary(
                BridgeState::ChatGptNotRunning,
                "ChatGPT is not running; it was not started automatically.",
                self.version.clone(),
                false,
            ));
        }
        let bundle = discover_chatgpt_bundle()?;
        terminate_processes(&processes).await?;
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        drop(listener);
        let status = Command::new("/usr/bin/open")
            .args([
                "-n",
                "-a",
                bundle.to_string_lossy().as_ref(),
                "--args",
                "--remote-debugging-address=127.0.0.1",
                &format!("--remote-debugging-port={port}"),
            ])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("LaunchServices could not restart ChatGPT.".into());
        }
        write_state_file(&self.state_file, port)?;
        self.port = Some(port);
        for _ in 0..80 {
            if is_loopback_debug_endpoint(port).await {
                return Ok(self.connect().await);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.port = None;
        Err("ChatGPT did not expose the local Bridge within 8 seconds.".into())
    }

    pub async fn restart_normal(&mut self, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Normal restart requires explicit confirmation.".into());
        }
        let processes = running_chatgpt_processes()?;
        if processes.is_empty() {
            return Ok(());
        }
        let bundle = discover_chatgpt_bundle()?;
        terminate_processes(&processes).await?;
        let status = Command::new("/usr/bin/open")
            .args(["-a", bundle.to_string_lossy().as_ref()])
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            self.port = None;
            let _ = std::fs::remove_file(&self.state_file);
            Ok(())
        } else {
            Err("LaunchServices could not restart ChatGPT normally.".into())
        }
    }
}

fn capability(id: &str, label: &str, supported: bool) -> Capability {
    Capability {
        id: id.into(),
        label: label.into(),
        status: if supported {
            CapabilityStatus::Supported
        } else {
            CapabilityStatus::Unsupported
        },
        reason: (!supported).then(|| "Native behavior was not found in this Renderer.".into()),
    }
}

fn is_hard_capability(id: &str) -> bool {
    REQUIRED_BASE_CAPABILITIES.contains(&id) || id.starts_with("keycap.")
}

fn layout_import_supported(layout: &Layout) -> bool {
    layout.validate_importable().is_ok()
}

pub(crate) fn capabilities_fully_supported(capabilities: &[Capability]) -> bool {
    let hard_capabilities: Vec<_> = capabilities
        .iter()
        .filter(|capability| is_hard_capability(&capability.id))
        .collect();
    let expected_count = REQUIRED_BASE_CAPABILITIES.len() + OFFICIAL_KEYCAP_IDS.len();
    if hard_capabilities.len() != expected_count
        || hard_capabilities
            .iter()
            .any(|capability| capability.status != CapabilityStatus::Supported)
    {
        return false;
    }
    REQUIRED_BASE_CAPABILITIES.iter().all(|required| {
        hard_capabilities
            .iter()
            .filter(|capability| capability.id == *required)
            .count()
            == 1
    }) && OFFICIAL_KEYCAP_IDS.iter().all(|id| {
        let expected = format!("keycap.{id}");
        hard_capabilities
            .iter()
            .filter(|capability| capability.id == expected)
            .count()
            == 1
    })
}

fn executed(capability: String) -> ActionResult {
    ActionResult {
        outcome: DispatchOutcome::Executed,
        capability: Some(capability),
    }
}

fn summary(
    state: BridgeState,
    message: &str,
    codex_version: Option<String>,
    fully_compatible: bool,
) -> BridgeSummary {
    BridgeSummary {
        state,
        message: message.into(),
        codex_version,
        fully_compatible,
    }
}

fn inject_request(template: &str, request: &Value) -> Result<String, String> {
    if template.matches("__REQUEST__").count() != 1 {
        return Err("Bundled Renderer script has an invalid request placeholder.".into());
    }
    Ok(template.replace("__REQUEST__", &request.to_string()))
}

fn select_main_target(targets: &[DebugTarget]) -> Option<&DebugTarget> {
    let candidates: Vec<_> = targets
        .iter()
        .filter(|target| {
            target.target_type == "page"
                && target.web_socket_debugger_url.is_some()
                && target.url.starts_with("app://")
        })
        .collect();
    let is_index = |target: &&DebugTarget| {
        Url::parse(&target.url)
            .ok()
            .is_some_and(|url| url.path() == "/index.html")
    };
    let auxiliary = |target: &&DebugTarget| {
        target.url.contains("avatar-overlay")
            || target.url.contains("composition-surface")
            || target.url.contains("initialRoute=")
    };
    candidates
        .iter()
        .find(|target| {
            is_index(target)
                && Url::parse(&target.url)
                    .ok()
                    .is_some_and(|url| url.query().is_none())
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|target| is_index(target) && !auxiliary(target))
        })
        .or_else(|| candidates.iter().find(|target| !auxiliary(target)))
        .copied()
}

async fn main_debug_target(port: u16) -> Result<DebugTarget, String> {
    let endpoint = format!("http://127.0.0.1:{port}/json");
    let targets: Vec<DebugTarget> = timeout(
        Duration::from_secs(2),
        reqwest::Client::new().get(endpoint).send(),
    )
    .await
    .map_err(|_| "cdp_http_timeout".to_string())?
    .map_err(|_| "cdp_http_unavailable".to_string())?
    .json()
    .await
    .map_err(|_| "cdp_target_invalid".to_string())?;
    select_main_target(&targets)
        .cloned()
        .ok_or_else(|| "main_renderer_missing".to_string())
}

async fn cdp_evaluate(port: u16, target: &DebugTarget, expression: &str) -> Result<Value, String> {
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| "main_renderer_missing".to_string())?;
    let url = Url::parse(websocket).map_err(|_| "cdp_websocket_invalid".to_string())?;
    if url.host_str() != Some("127.0.0.1") || url.port() != Some(port) {
        return Err("cdp_non_loopback_rejected".into());
    }
    let (mut socket, _) = timeout(Duration::from_secs(2), connect_async(websocket))
        .await
        .map_err(|_| "cdp_websocket_timeout".to_string())?
        .map_err(|_| "cdp_websocket_unavailable".to_string())?;
    socket
        .send(Message::Text(
            json!({
                "id": 1,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|_| "cdp_send_failed".to_string())?;
    let deadline = Duration::from_secs(4);
    loop {
        let message = timeout(deadline, socket.next())
            .await
            .map_err(|_| "cdp_evaluate_timeout".to_string())?
            .ok_or_else(|| "renderer_disconnected".to_string())?
            .map_err(|_| "renderer_disconnected".to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let response: Value =
            serde_json::from_str(&text).map_err(|_| "cdp_response_invalid".to_string())?;
        if response.get("id").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        if response.get("error").is_some() || response.pointer("/result/exceptionDetails").is_some()
        {
            return Err("renderer_evaluation_failed".into());
        }
        return Ok(response
            .pointer("/result/result/value")
            .cloned()
            .unwrap_or(Value::Null));
    }
}

async fn is_loopback_debug_endpoint(port: u16) -> bool {
    timeout(
        Duration::from_millis(500),
        reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/json/version"))
            .send(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|response| response.status().is_success())
}

fn running_chatgpt_processes() -> Result<Vec<ChatGptProcess>, String> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Process inspection failed.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (pid, command) = line.split_once(char::is_whitespace)?;
            if !command.contains("/ChatGPT.app/Contents/MacOS/ChatGPT") {
                return None;
            }
            Some(ChatGptProcess {
                pid: pid.parse().ok()?,
                command: command.trim().into(),
            })
        })
        .collect())
}

fn port_from_processes(processes: &[ChatGptProcess]) -> Option<u16> {
    processes.iter().find_map(|process| {
        if !process
            .command
            .contains("--remote-debugging-address=127.0.0.1")
        {
            return None;
        }
        process.command.split_whitespace().find_map(|argument| {
            argument
                .strip_prefix("--remote-debugging-port=")?
                .parse()
                .ok()
        })
    })
}

fn discover_chatgpt_bundle() -> Result<PathBuf, String> {
    let candidates = [
        PathBuf::from("/Applications/ChatGPT.app"),
        PathBuf::from("/System/Applications/ChatGPT.app"),
    ];
    candidates
        .into_iter()
        .find(|path| {
            path.exists()
                && bundle_value(path, "CFBundleIdentifier")
                    .is_ok_and(|identifier| identifier == CHATGPT_BUNDLE_ID)
        })
        .ok_or_else(|| "A signed ChatGPT.app bundle was not found.".into())
}

fn bundle_value(bundle: &Path, key: &str) -> Result<String, String> {
    let info = bundle.join("Contents/Info.plist");
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(info)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("ChatGPT bundle metadata {key} is unavailable."));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

async fn terminate_processes(processes: &[ChatGptProcess]) -> Result<(), String> {
    for process in processes {
        let status = Command::new("/bin/kill")
            .args(["-TERM", &process.pid.to_string()])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!(
                "Could not terminate ChatGPT process {}.",
                process.pid
            ));
        }
    }
    for _ in 0..80 {
        let remaining = running_chatgpt_processes()?;
        if remaining.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("ChatGPT did not exit cleanly within 8 seconds.".into())
}

fn write_state_file(path: &Path, port: u16) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Bridge state path is invalid.".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".bridge-{}.tmp", Uuid::new_v4()));
    let state = BridgeStateFile {
        port,
        generation: Uuid::new_v4().to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

pub fn same_thread_identity(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    if left == right {
        return true;
    }
    fn uuid_tail(value: &str) -> Option<String> {
        let tail = value.rsplit(':').next()?;
        if tail.len() == 36
            && tail.chars().enumerate().all(|(index, character)| {
                matches!(index, 8 | 13 | 18 | 23) && character == '-'
                    || !matches!(index, 8 | 13 | 18 | 23) && character.is_ascii_hexdigit()
            })
        {
            Some(tail.to_ascii_lowercase())
        } else {
            None
        }
    }
    uuid_tail(left).is_some_and(|uuid| Some(uuid) == uuid_tail(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> DebugTarget {
        DebugTarget {
            id: "target-1".into(),
            target_type: "page".into(),
            url: url.into(),
            web_socket_debugger_url: Some("ws://127.0.0.1:9000/devtools/page/1".into()),
        }
    }

    fn complete_hard_capabilities() -> Vec<Capability> {
        REQUIRED_BASE_CAPABILITIES
            .iter()
            .map(|id| capability(id, id, true))
            .chain(
                OFFICIAL_KEYCAP_IDS
                    .iter()
                    .map(|id| capability(&format!("keycap.{id}"), id, true)),
            )
            .collect()
    }

    #[test]
    fn main_target_excludes_auxiliary_and_query_renderers() {
        let targets = vec![
            target("app://openai/avatar-overlay/index.html"),
            target("app://openai/index.html?initialRoute=voice"),
            target("app://openai/index.html"),
        ];
        assert_eq!(
            select_main_target(&targets).unwrap().url,
            "app://openai/index.html"
        );
    }

    #[test]
    fn thread_identity_accepts_host_prefixes_but_not_guesses() {
        let uuid = "01234567-89ab-cdef-0123-456789abcdef";
        assert!(same_thread_identity(
            Some(&format!("darwin:host:{uuid}")),
            Some(&format!("local:{uuid}"))
        ));
        assert!(!same_thread_identity(Some("one"), Some("two")));
    }

    #[test]
    fn bundled_scripts_have_one_typed_request_seam_and_no_remote_code() {
        assert_eq!(BINDING_SCRIPT.matches("__REQUEST__").count(), 1);
        assert_eq!(ACTION_SCRIPT.matches("__REQUEST__").count(), 1);
        for script in [SNAPSHOT_SCRIPT, PROBE_SCRIPT, BINDING_SCRIPT, ACTION_SCRIPT] {
            assert!(!script.contains("http://"));
            assert!(!script.contains("https://"));
            assert!(!script.contains("eval("));
        }
        assert!(SNAPSHOT_SCRIPT.contains(r#": "unknown""#));
        assert!(
            SNAPSHOT_SCRIPT
                .find("const action = binding.action ?? binding")
                .unwrap()
                < SNAPSHOT_SCRIPT
                    .find("if (allowKeycap && typeof binding.keycapId")
                    .unwrap()
        );
        assert!(SNAPSHOT_SCRIPT.contains(r#"type: "unsupported""#));
        for id in OFFICIAL_KEYCAP_IDS {
            assert!(PROBE_SCRIPT.contains(&format!("\"{id}\"")));
        }
    }

    #[test]
    fn empty_capability_report_is_not_fully_supported() {
        assert!(!capabilities_fully_supported(&[]));
    }

    #[test]
    fn complete_capability_report_requires_every_hard_capability() {
        let complete = complete_hard_capabilities();
        assert!(capabilities_fully_supported(&complete));

        let mut missing_one = complete.clone();
        missing_one.retain(|capability| capability.id != "mic-state");
        assert!(!capabilities_fully_supported(&missing_one));

        let mut unsupported = complete;
        unsupported
            .iter_mut()
            .find(|capability| capability.id == "encoder")
            .unwrap()
            .status = CapabilityStatus::Unsupported;
        assert!(!capabilities_fully_supported(&unsupported));
    }

    #[test]
    fn unsupported_import_position_does_not_disable_safe_import_preview() {
        let mut layout = Layout::official_defaults();
        layout.command_slots.insert(
            crate::model::CommandSlot::Act06,
            Binding::Unsupported {
                label: "Unknown keycap FUTURE".into(),
            },
        );

        assert!(layout.validate().is_err());
        assert!(layout_import_supported(&layout));
    }

    #[test]
    fn renderer_target_change_invalidates_previous_capabilities() {
        let directory = tempfile::tempdir().unwrap();
        let mut adapter = CodexDesktopAdapter::new(directory.path().join("bridge-state.json"));
        adapter.target_generation = Some("target-1".into());
        adapter.capability_generation = Some("target-1".into());
        adapter.capabilities = complete_hard_capabilities();

        let result = adapter.observe_target_generation("target-2", false);

        assert_eq!(result.unwrap_err(), "renderer_generation_changed");
        assert!(adapter.capabilities.is_empty());
        assert!(adapter.capability_generation.is_none());
        assert_eq!(adapter.target_generation.as_deref(), Some("target-2"));
        assert!(adapter.take_control_reset_required());
        assert!(!adapter.take_control_reset_required());
    }

    #[test]
    fn probe_failure_after_a_snapshot_cannot_reuse_previous_capabilities() {
        let directory = tempfile::tempdir().unwrap();
        let mut adapter = CodexDesktopAdapter::new(directory.path().join("bridge-state.json"));
        adapter.last_renderer_snapshot = Some(
            serde_json::from_value(json!({
                "agents": (0..6).map(|slot| json!({
                    "slot": slot,
                    "threadKey": null,
                    "title": null,
                    "nativeStatus": "idle",
                    "selected": false,
                    "contextUsedPercent": null,
                })).collect::<Vec<_>>(),
                "agentSource": "priority",
                "activeThread": null,
                "importedLayout": Layout::official_defaults(),
                "theme": "dark",
                "usage": null,
                "micState": "idle",
            }))
            .unwrap(),
        );
        adapter.target_generation = Some("target-1".into());
        adapter.capability_generation = Some("target-1".into());
        adapter.capabilities = complete_hard_capabilities();

        adapter.begin_probe();

        assert!(adapter.last_renderer_snapshot.is_some());
        assert!(adapter.capabilities.is_empty());
        assert!(adapter.capability_generation.is_none());
        assert!(!capabilities_fully_supported(adapter.capabilities()));
    }

    #[tokio::test]
    #[ignore = "requires a running ChatGPT Desktop with loopback CDP enabled"]
    async fn live_renderer_snapshot_matches_the_typed_contract() {
        let directory = tempfile::tempdir().unwrap();
        let mut adapter = CodexDesktopAdapter::new(directory.path().join("bridge-state.json"));
        let bridge = adapter.connect().await;
        assert!(matches!(
            bridge.state,
            BridgeState::AwaitingSnapshot | BridgeState::Degraded
        ));
        let snapshot = adapter.refresh().await.unwrap();
        assert_eq!(snapshot.normalized_agents().len(), 6);
        assert_eq!(snapshot.imported_layout.command_slots.len(), 6);
        assert_eq!(snapshot.imported_layout.joystick.len(), 4);
        assert_eq!(snapshot.mic_state, Some(MicState::Idle));
        assert_eq!(
            adapter
                .capabilities()
                .iter()
                .filter(|capability| capability.id.starts_with("keycap."))
                .count(),
            30
        );
        assert!(
            capabilities_fully_supported(adapter.capabilities()),
            "{:#?}",
            adapter.capabilities()
        );
    }
}
