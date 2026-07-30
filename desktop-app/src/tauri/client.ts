import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSnapshot,
  Binding,
  CommandSlot,
  ConfigResult,
  Direction,
  EncoderMode,
  InputEvent,
} from "../model";

export type ConfigCommand =
  | { type: "setCommand"; slot: CommandSlot; binding: Binding }
  | { type: "setJoystick"; direction: Direction; binding: Binding }
  | { type: "setEncoderMode"; mode: EncoderMode }
  | { type: "previewImport" }
  | { type: "commitImport"; revision: string }
  | { type: "undo" }
  | { type: "restoreDefaults"; confirmed: boolean }
  | {
      type: "setPreferences";
      launchAtLogin?: boolean;
      onboardingComplete?: boolean;
    };

export type BridgeCommand =
  | { type: "connect" }
  | { type: "probe" }
  | { type: "restart"; confirmed: boolean };

export type AppCommand =
  | { type: "clearDiagnostics" }
  | { type: "exportDiagnostics" }
  | { type: "consumeReset"; confirmed: true }
  | { type: "quit" }
  | { type: "quitAndRestartNormal"; confirmed: boolean };

export interface AgentBridgeClient {
  snapshot(): Promise<AppSnapshot>;
  input(event: InputEvent): Promise<unknown>;
  configure(command: ConfigCommand): Promise<ConfigResult>;
  bridge(command: BridgeCommand): Promise<AppSnapshot>;
  app(command: AppCommand): Promise<{ path?: string; snapshot?: AppSnapshot }>;
  subscribe(listener: (snapshot: AppSnapshot) => void): Promise<UnlistenFn>;
}

export const tauriClient: AgentBridgeClient = {
  snapshot: () => invoke<AppSnapshot>("snapshot"),
  input: (event) => invoke("input", { event }),
  configure: (command) => invoke<ConfigResult>("configure", { command }),
  bridge: (command) => invoke<AppSnapshot>("bridge", { command }),
  app: (command) => invoke("app", { command }),
  subscribe: async (listener) =>
    await listen<AppSnapshot>("state_changed", (event) => listener(event.payload)),
};
