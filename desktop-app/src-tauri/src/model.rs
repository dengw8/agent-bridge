use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const OFFICIAL_KEYCAP_IDS: [&str; 30] = [
    "FAST", "APPR", "REJ", "SPLIT", "MIC", "CODEX", "BUG", "OAI", "TERM", "DWN", "DEL", "NEW",
    "NAV", "MAGIC", "DIFF", "PLAY", "GIT", "BRCH", "MRG", "PR", "PAINT", "LAB", "PARTY", "TIME",
    "MIND+", "MIND-", "SETUP", "FOLD", "UPL", "APPS",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CommandSlot {
    #[serde(rename = "ACT06")]
    Act06,
    #[serde(rename = "ACT07")]
    Act07,
    #[serde(rename = "ACT08")]
    Act08,
    #[serde(rename = "ACT09")]
    Act09,
    #[serde(rename = "ACT10_ACT11")]
    Act10Act11,
    #[serde(rename = "ACT12")]
    Act12,
}

impl CommandSlot {
    pub const ALL: [Self; 6] = [
        Self::Act06,
        Self::Act07,
        Self::Act08,
        Self::Act09,
        Self::Act10Act11,
        Self::Act12,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Act06 => "ACT06",
            Self::Act07 => "ACT07",
            Self::Act08 => "ACT08",
            Self::Act09 => "ACT09",
            Self::Act10Act11 => "ACT10_ACT11",
            Self::Act12 => "ACT12",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Up, Self::Right, Self::Down, Self::Left];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Right => "right",
            Self::Down => "down",
            Self::Left => "left",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderMode {
    #[serde(rename = "composer-navigation")]
    ComposerNavigation,
    #[serde(rename = "reasoning")]
    Reasoning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Binding {
    Keycap { id: String },
    Command { id: String },
    Skill { identity: SkillIdentity },
    Unsupported { label: String },
    None,
}

impl Binding {
    pub fn label(&self) -> String {
        match self {
            Self::Keycap { id } | Self::Command { id } => id.clone(),
            Self::Skill { identity } => format!("${}", identity.name),
            Self::Unsupported { label } => label.clone(),
            Self::None => "None".into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Keycap { id } if !OFFICIAL_KEYCAP_IDS.contains(&id.as_str()) => {
                Err(format!("Unsupported keycap: {id}"))
            }
            Self::Command { id } if id.trim().is_empty() => Err("Command ID is empty.".into()),
            Self::Skill { identity }
                if identity.name.trim().is_empty() || identity.path.trim().is_empty() =>
            {
                Err("Skill identity is incomplete.".into())
            }
            Self::Unsupported { label } => Err(format!("Unsupported imported binding: {label}")),
            _ => Ok(()),
        }
    }

    fn validate_importable(&self) -> Result<(), String> {
        match self {
            Self::Unsupported { label } if label.trim().is_empty() => {
                Err("Unsupported imported binding has no label.".into())
            }
            Self::Unsupported { .. } => Ok(()),
            binding => binding.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillIdentity {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub command_slots: BTreeMap<CommandSlot, Binding>,
    pub joystick: BTreeMap<Direction, Binding>,
    pub encoder_mode: EncoderMode,
}

impl Layout {
    pub fn official_defaults() -> Self {
        let mut command_slots = BTreeMap::new();
        for (slot, id) in CommandSlot::ALL
            .into_iter()
            .zip(["FAST", "APPR", "REJ", "SPLIT", "MIC", "CODEX"])
        {
            command_slots.insert(slot, Binding::Keycap { id: id.into() });
        }
        let mut joystick = BTreeMap::new();
        for (direction, id) in Direction::ALL.into_iter().zip([
            "composer.togglePlanMode",
            "navigateForward",
            "toggleSidebar",
            "navigateBack",
        ]) {
            joystick.insert(direction, Binding::Command { id: id.into() });
        }
        Self {
            command_slots,
            joystick,
            encoder_mode: EncoderMode::ComposerNavigation,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_shape()?;
        for binding in self.command_slots.values().chain(self.joystick.values()) {
            binding.validate()?;
        }
        Ok(())
    }

    pub fn validate_importable(&self) -> Result<(), String> {
        self.validate_shape()?;
        for binding in self.command_slots.values().chain(self.joystick.values()) {
            binding.validate_importable()?;
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), String> {
        if self.command_slots.len() != 6
            || CommandSlot::ALL
                .iter()
                .any(|slot| !self.command_slots.contains_key(slot))
        {
            return Err("Layout must contain all six command slots.".into());
        }
        if self.joystick.len() != 4
            || Direction::ALL
                .iter()
                .any(|direction| !self.joystick.contains_key(direction))
        {
            return Err("Layout must contain all four joystick directions.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub launch_at_login: bool,
    pub onboarding_complete: bool,
    #[serde(default)]
    pub theme_mode: ThemePreference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Initialization {
    pub source: InitializationSource,
    pub imported_from_codex_version: Option<String>,
    pub imported_at: Option<String>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InitializationSource {
    CodexMicro,
    OfficialDefaults,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub schema_version: u8,
    pub layout: Layout,
    pub initialization: Initialization,
    pub preferences: Preferences,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            layout: Layout::official_defaults(),
            initialization: Initialization {
                source: InitializationSource::OfficialDefaults,
                imported_from_codex_version: None,
                imported_at: None,
                complete: false,
            },
            preferences: Preferences::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentSource {
    Recent,
    Pinned,
    Priority,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentVisualStatus {
    Empty,
    Idle,
    Thinking,
    Complete,
    Input,
    Error,
}

pub fn visual_status(native: &str) -> AgentVisualStatus {
    match native {
        "off" => AgentVisualStatus::Empty,
        "working" | "thinking" => AgentVisualStatus::Thinking,
        "unread" | "complete" | "completed" | "done" => AgentVisualStatus::Complete,
        "approval" | "awaiting-approval" | "awaiting-response" => AgentVisualStatus::Input,
        "error" => AgentVisualStatus::Error,
        _ => AgentVisualStatus::Idle,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSlot {
    pub slot: u8,
    pub thread_key: Option<String>,
    pub title: Option<String>,
    pub status: AgentVisualStatus,
    pub native_status: String,
    pub selected: bool,
    pub context_used_percent: Option<f64>,
    pub stale: bool,
}

impl AgentSlot {
    pub fn empty(slot: u8) -> Self {
        Self {
            slot,
            thread_key: None,
            title: None,
            status: AgentVisualStatus::Empty,
            native_status: "off".into(),
            selected: false,
            context_used_percent: None,
            stale: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
    TemporarilyUnavailable,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
    pub label: String,
    pub status: CapabilityStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BridgeState {
    ChatGptNotRunning,
    RestartRequired,
    Connecting,
    AwaitingSnapshot,
    Ready,
    Degraded,
    Incompatible,
    Disconnected,
    Restarting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeSummary {
    pub state: BridgeState,
    pub message: String,
    pub codex_version: Option<String>,
    pub fully_compatible: bool,
}

impl Default for BridgeSummary {
    fn default() -> Self {
        Self {
            state: BridgeState::Disconnected,
            message: "Bridge has not connected yet.".into(),
            codex_version: None,
            fully_compatible: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicState {
    Idle,
    Recording,
    Processing,
    Ready,
    Locked,
    Stopping,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    pub kind: String,
    pub remaining_percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub windows: Vec<UsageWindow>,
    pub reset_credits_available: Option<i64>,
    pub reset_credits_applicable: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveThread {
    pub thread_key: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEntry {
    pub at: String,
    pub category: String,
    pub outcome: String,
    pub duration_ms: Option<u64>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DispatchOutcome {
    Executed,
    Unavailable,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchReceipt {
    pub request_id: String,
    pub intent_kind: String,
    pub outcome: DispatchOutcome,
    pub duration_ms: u64,
    pub capability: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub observed_at_ms: u64,
    pub bridge: BridgeSummary,
    pub hardware_connection: HardwareConnectionState,
    pub capabilities: Vec<Capability>,
    pub agents: Vec<AgentSlot>,
    pub agent_source: AgentSource,
    pub active_thread: Option<ActiveThread>,
    pub layout: Layout,
    pub imported_layout: Option<Layout>,
    pub initialization_source: InitializationSource,
    pub mic_state: Option<MicState>,
    pub theme: ThemeMode,
    pub usage: Option<UsageSnapshot>,
    pub preferences: Preferences,
    pub diagnostics: Vec<DiagnosticEntry>,
    pub last_receipt: Option<DispatchReceipt>,
}

impl AppSnapshot {
    pub fn initial(config: &AppConfig) -> Self {
        Self {
            observed_at_ms: now_ms(),
            bridge: BridgeSummary::default(),
            hardware_connection: HardwareConnectionState::Disconnected,
            capabilities: Vec::new(),
            agents: (0..6).map(AgentSlot::empty).collect(),
            agent_source: AgentSource::Priority,
            active_thread: None,
            layout: config.layout.clone(),
            imported_layout: None,
            initialization_source: config.initialization.source,
            mic_state: None,
            theme: ThemeMode::Dark,
            usage: None,
            preferences: config.preferences.clone(),
            diagnostics: Vec::new(),
            last_receipt: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HardwareConnectionState {
    Disconnected,
    Connecting,
    PermissionDenied,
    Incompatible,
    Available,
    Unavailable,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSource {
    VirtualUi,
    Hardware(TrustedHardwareSource),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrustedHardwareSource {
    pub(crate) id: u64,
    pub(crate) transport: HardwareTransport,
    pub(crate) generation: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HardwareTransport {
    Fake,
    Usb,
    Ble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VirtualInputSource {
    VirtualUi,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlId {
    Agent { slot: u8 },
    Command { slot: CommandSlot },
    Keycap { id: String },
    Joystick { direction: Direction },
    EncoderPress,
    EncoderRotate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputPhase {
    Pressed,
    Released,
    Entered,
    Centered,
    Rotated { steps: i16 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub source: InputSource,
    pub control: ControlId,
    pub phase: InputPhase,
    pub sequence: u64,
    pub monotonic_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VirtualInputEvent {
    pub source: VirtualInputSource,
    pub control: ControlId,
    pub phase: InputPhase,
    pub sequence: u64,
    pub monotonic_ms: f64,
}

impl From<VirtualInputEvent> for InputEvent {
    fn from(event: VirtualInputEvent) -> Self {
        Self {
            source: InputSource::VirtualUi,
            control: event.control,
            phase: event.phase,
            sequence: event.sequence,
            monotonic_ms: event.monotonic_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    AgentEdge {
        slot: u8,
        thread_key: Option<String>,
        pressed: bool,
    },
    ExecuteBinding {
        slot: Option<CommandSlot>,
        binding: Binding,
        pressed: bool,
    },
    JoystickEdge {
        direction: Direction,
        active: bool,
        binding: Binding,
    },
    EncoderEdge {
        mode: EncoderMode,
        pressed: bool,
    },
    EncoderRotate {
        mode: EncoderMode,
        steps: i16,
    },
}

impl Intent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AgentEdge { .. } => "agentEdge",
            Self::ExecuteBinding { .. } => "executeBinding",
            Self::JoystickEdge { .. } => "joystickEdge",
            Self::EncoderEdge { .. } => "encoderEdge",
            Self::EncoderRotate { .. } => "encoderRotate",
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::MicState;

    #[test]
    fn microphone_states_have_distinct_typed_wire_values() {
        for (wire, state) in [
            ("idle", MicState::Idle),
            ("recording", MicState::Recording),
            ("processing", MicState::Processing),
            ("ready", MicState::Ready),
            ("locked", MicState::Locked),
            ("stopping", MicState::Stopping),
            ("unknown", MicState::Unknown),
        ] {
            assert_eq!(
                serde_json::from_str::<MicState>(&format!(r#""{wire}""#)).unwrap(),
                state
            );
        }
    }
}
