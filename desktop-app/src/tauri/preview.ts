import { KEYCAPS, type AppSnapshot, type Layout } from "../model";
import type {
  AgentBridgeClient,
  AppCommand,
  BridgeCommand,
  ConfigCommand,
} from "./client";

const officialLayout: Layout = {
  commandSlots: {
    ACT06: { type: "keycap", id: "FAST" },
    ACT07: { type: "keycap", id: "APPR" },
    ACT08: { type: "keycap", id: "REJ" },
    ACT09: { type: "keycap", id: "SPLIT" },
    ACT10_ACT11: { type: "keycap", id: "MIC" },
    ACT12: { type: "keycap", id: "CODEX" },
  },
  joystick: {
    up: { type: "command", id: "composer.togglePlanMode" },
    right: { type: "command", id: "navigateForward" },
    down: { type: "command", id: "toggleSidebar" },
    left: { type: "command", id: "navigateBack" },
  },
  encoderMode: "composer-navigation",
};
const clone = <T,>(value: T): T => structuredClone(value);
let undoLayout: Layout | undefined;
let current: AppSnapshot = {
  observedAtMs: Date.now(),
  bridge: {
    state: "ready",
    message: "Native Codex controls are ready.",
    codexVersion: "26.721.81911",
    fullyCompatible: true,
  },
  capabilities: [
    { id: "snapshot", label: "Native status snapshot", status: "supported" },
    { id: "agent-slots", label: "Six native Agent slots", status: "supported" },
    { id: "native-binding-override", label: "Independent native Layout", status: "supported" },
    { id: "joystick", label: "Native analog stick handler", status: "supported" },
    { id: "encoder", label: "Native encoder gestures", status: "supported" },
    ...KEYCAPS.map(([id]) => ({
      id: `keycap.${id}`,
      label: `Keycap ${id}`,
      status: "supported" as const,
    })),
  ],
  agents: Array.from({ length: 6 }, (_, slot) => ({
    slot,
    threadKey: `local:00000000-0000-0000-0000-00000000000${slot}`,
    title: ["Bridge backend", "Renderer probe", "Configuration", "Interface parity", "Tests", "Release check"][slot],
    status: (["thinking", "idle", "complete", "input", "idle", "error"] as const)[slot],
    nativeStatus: (["working", "idle", "complete", "approval", "idle", "error"] as const)[slot],
    selected: slot === 0,
    contextUsedPercent: 12 + slot * 11,
    stale: false,
  })),
  agentSource: "recent",
  activeThread: {
    threadKey: "local:00000000-0000-0000-0000-000000000000",
    title: "Bridge backend",
  },
  layout: clone(officialLayout),
  importedLayout: clone(officialLayout),
  initializationSource: "codex-micro",
  micState: "idle",
  theme: "dark",
  usage: {
    windows: [
      { id: "five-hour", kind: "five-hour", remainingPercent: 73, resetsAt: null },
      { id: "weekly", kind: "weekly", remainingPercent: 41, resetsAt: null },
    ],
    resetCreditsAvailable: 2,
    resetCreditsApplicable: 1,
  },
  preferences: { launchAtLogin: false, onboardingComplete: true },
  diagnostics: [
    { at: new Date().toISOString(), category: "bridge", outcome: "complete", durationMs: 18 },
    { at: new Date().toISOString(), category: "probe", outcome: "supported", durationMs: 42 },
  ],
};

async function configure(command: ConfigCommand) {
  if (command.type === "previewImport") {
    return {
      snapshot: clone(current),
      importRevision: "preview-only-revision",
      preview: [{
        position: "ACT06",
        current: current.layout.commandSlots.ACT06.type === "keycap"
          ? current.layout.commandSlots.ACT06.id
          : "Custom",
        imported: "FAST",
        result: "FAST",
      }],
    };
  }
  if (command.type === "commitImport") {
    undoLayout = clone(current.layout);
    current.layout = clone(officialLayout);
  } else if (command.type === "setCommand") {
    undoLayout = clone(current.layout);
    current.layout.commandSlots[command.slot] = clone(command.binding);
  } else if (command.type === "setJoystick") {
    undoLayout = clone(current.layout);
    current.layout.joystick[command.direction] = clone(command.binding);
  } else if (command.type === "setEncoderMode") {
    undoLayout = clone(current.layout);
    current.layout.encoderMode = command.mode;
  } else if (command.type === "restoreDefaults") {
    undoLayout = clone(current.layout);
    current.layout = clone(officialLayout);
  } else if (command.type === "undo" && undoLayout) {
    const next = clone(current.layout);
    current.layout = undoLayout;
    undoLayout = next;
  } else if (command.type === "setPreferences") {
    current.preferences = {
      launchAtLogin: command.launchAtLogin ?? current.preferences.launchAtLogin,
      onboardingComplete: command.onboardingComplete ?? current.preferences.onboardingComplete,
    };
  }
  current.observedAtMs = Date.now();
  return { snapshot: clone(current) };
}

export const previewClient: AgentBridgeClient = {
  snapshot: async () => clone(current),
  input: async () => ({ outcome: "executed" }),
  configure,
  bridge: async (_command: BridgeCommand) => clone(current),
  app: async (command: AppCommand) => {
    if (command.type === "clearDiagnostics") {
      current.diagnostics = [];
      return { snapshot: clone(current) };
    }
    if (command.type === "exportDiagnostics") {
      return { path: "/tmp/agent-bridge-diagnostics-preview.json" };
    }
    return {};
  },
  subscribe: async () => () => undefined,
};
