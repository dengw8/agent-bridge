export const COMMAND_SLOTS = [
  "ACT06",
  "ACT07",
  "ACT08",
  "ACT09",
  "ACT10_ACT11",
  "ACT12",
] as const;
export type CommandSlot = (typeof COMMAND_SLOTS)[number];

export const DIRECTIONS = ["up", "right", "down", "left"] as const;
export type Direction = (typeof DIRECTIONS)[number];
export type EncoderMode = "composer-navigation" | "reasoning";

export const KEYCAPS = [
  ["FAST", "Fast Mode"],
  ["APPR", "Approve"],
  ["REJ", "Reject"],
  ["SPLIT", "Fork Chat"],
  ["MIC", "Push to Talk"],
  ["CODEX", "Submit"],
  ["BUG", "Feedback"],
  ["OAI", "OpenAI Docs"],
  ["TERM", "Terminal"],
  ["DWN", "Copy Chat Markdown"],
  ["DEL", "Archive Chat"],
  ["NEW", "New Task"],
  ["NAV", "Browser"],
  ["MAGIC", "Pin / Unpin"],
  ["DIFF", "Review"],
  ["PLAY", "Environment Action"],
  ["GIT", "Git Commit"],
  ["BRCH", "Draft Pull Request"],
  ["MRG", "Merge Pull Request"],
  ["PR", "Create Pull Request"],
  ["PAINT", "Add Photos"],
  ["LAB", "Settings"],
  ["PARTY", "Side Chat"],
  ["TIME", "Manage Tasks"],
  ["MIND+", "Increase Reasoning"],
  ["MIND-", "Decrease Reasoning"],
  ["SETUP", "Settings"],
  ["FOLD", "Open Folder"],
  ["UPL", "Add Files"],
  ["APPS", "Skills"],
] as const;
export type KeycapId = (typeof KEYCAPS)[number][0];

export type Binding =
  | { type: "keycap"; id: KeycapId }
  | { type: "command"; id: string }
  | { type: "skill"; identity: { name: string; path: string } }
  | { type: "unsupported"; label: string }
  | { type: "none" };

export type Layout = {
  commandSlots: Record<CommandSlot, Binding>;
  joystick: Record<Direction, Binding>;
  encoderMode: EncoderMode;
};

export type AgentSlot = {
  slot: number;
  threadKey: string | null;
  title: string | null;
  status: "empty" | "idle" | "thinking" | "complete" | "input" | "error";
  nativeStatus: string;
  selected: boolean;
  contextUsedPercent?: number;
  stale: boolean;
};

export type CapabilityStatus =
  | "supported"
  | "unsupported"
  | "temporarilyUnavailable"
  | "notApplicable";

export type Capability = {
  id: string;
  label: string;
  status: CapabilityStatus;
  reason?: string;
};

export type BridgeState =
  | "chatGptNotRunning"
  | "restartRequired"
  | "connecting"
  | "awaitingSnapshot"
  | "ready"
  | "degraded"
  | "incompatible"
  | "disconnected"
  | "restarting";

export type DiagnosticEntry = {
  at: string;
  category: string;
  outcome: string;
  durationMs?: number;
  code?: string;
};

export type UsageWindow = {
  id: string;
  kind: "five-hour" | "weekly" | "other";
  remainingPercent: number;
  resetsAt: number | null;
};

export type ThemePreference = "system" | "light" | "dark";

export type AppSnapshot = {
  observedAtMs: number;
  bridge: {
    state: BridgeState;
    message: string;
    codexVersion?: string;
    fullyCompatible: boolean;
  };
  hardwareConnection: "disconnected" | "connecting" | "permissionDenied" | "incompatible" | "available" | "unavailable";
  capabilities: Capability[];
  agents: AgentSlot[];
  agentSource: "recent" | "pinned" | "priority" | "custom";
  activeThread?: { threadKey: string; title?: string };
  layout: Layout;
  importedLayout?: Layout;
  initializationSource: "codex-micro" | "official-defaults";
  micState?: "idle" | "recording" | "processing" | "ready" | "locked" | "stopping" | "unknown";
  theme: "light" | "dark";
  usage?: {
    windows: UsageWindow[];
    resetCreditsAvailable: number | null;
    resetCreditsApplicable: number | null;
  };
  preferences: {
    launchAtLogin: boolean;
    onboardingComplete: boolean;
    themeMode: ThemePreference;
  };
  diagnostics: DiagnosticEntry[];
  lastReceipt?: {
    outcome: "executed" | "unavailable" | "unsupported" | "failed";
    intentKind: string;
    durationMs: number;
    errorCode?: string;
  };
};

export type Control =
  | { type: "agent"; slot: number }
  | { type: "command"; slot: CommandSlot }
  | { type: "keycap"; id: KeycapId }
  | { type: "joystick"; direction: Direction }
  | { type: "encoderPress" }
  | { type: "encoderRotate" };

export type InputPhase =
  | { type: "pressed" }
  | { type: "released" }
  | { type: "entered" }
  | { type: "centered" }
  | { type: "rotated"; steps: number };

export type InputEvent = {
  source: "virtualUi";
  control: Control;
  phase: InputPhase;
  sequence: number;
  monotonicMs: number;
};

export type ImportPreviewItem = {
  position: string;
  current: string;
  imported: string;
  result: string;
};

export type ConfigResult = {
  snapshot: AppSnapshot;
  importRevision?: string;
  preview?: ImportPreviewItem[];
};
