import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import App from "./App";
import { COMMAND_SLOTS, type AppSnapshot } from "./model";
import type {
  AgentBridgeClient,
  AppCommand,
  BridgeCommand,
  ConfigCommand,
} from "./tauri/client";

function snapshot(): AppSnapshot {
  return {
    observedAtMs: Date.now(),
    bridge: {
      state: "ready",
      message: "All native controls verified.",
      codexVersion: "26.721.81911",
      fullyCompatible: true,
    },
    capabilities: [
      ...[
        "snapshot",
        "agent-slots",
        "native-binding-override",
        "mic-phase",
        "mic-state",
        "joystick",
        "encoder",
        "layout-import",
      ].map((id) => ({ id, label: id, status: "supported" as const })),
      ...["FAST", "APPR", "REJ", "SPLIT", "MIC", "CODEX", "TERM", "DIFF", "GIT", "PR", "UPL", "APPS"]
        .map((id) => ({ id: `keycap.${id}`, label: id, status: "supported" as const })),
    ],
    agents: Array.from({ length: 6 }, (_, slot) => ({
      slot,
      threadKey: `local:thread-${slot}`,
      title: `Task ${slot + 1}`,
      status: slot === 0 ? "thinking" as const : "idle" as const,
      nativeStatus: slot === 0 ? "working" : "idle",
      selected: slot === 1,
      stale: false,
    })),
    agentSource: "priority",
    activeThread: { threadKey: "local:thread-1", title: "Task 2" },
    layout: {
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
    },
    initializationSource: "codex-micro",
    micState: "idle",
    theme: "dark",
    preferences: {
      launchAtLogin: false,
      onboardingComplete: true,
      themeMode: "system",
    },
    diagnostics: [],
  };
}

function fakeClient(value = snapshot()) {
  let current = structuredClone(value);
  let previous: AppSnapshot | undefined;
  const input = vi.fn(async (_event: Parameters<AgentBridgeClient["input"]>[0]) => ({}));
  const configure = vi.fn(async (command: ConfigCommand) => {
    if (command.type === "setCommand") {
      previous = structuredClone(current);
      current.layout.commandSlots[command.slot] = command.binding;
    } else if (command.type === "setJoystick") {
      if (command.binding.type === "keycap") {
        throw new Error("Joystick mappings must be a command, skill, or None.");
      }
      previous = structuredClone(current);
      current.layout.joystick[command.direction] = command.binding;
    } else if (command.type === "undo" && previous) {
      current = previous;
      previous = undefined;
    } else if (command.type === "setPreferences") {
      previous = structuredClone(current);
      if (command.launchAtLogin != null) {
        current.preferences.launchAtLogin = command.launchAtLogin;
      }
      if (command.onboardingComplete != null) {
        current.preferences.onboardingComplete = command.onboardingComplete;
      }
      if (command.themeMode != null) {
        current.preferences.themeMode = command.themeMode;
      }
    }
    return {
      snapshot: structuredClone(current),
      ...(command.type === "previewImport"
        ? {
            importRevision: "revision-1",
            preview: [{
              position: "ACT06",
              current: "TERM",
              imported: "FAST",
              result: "Will change to FAST",
            }],
          }
      : {}),
    };
  });
  const bridge = vi.fn(async (_command: BridgeCommand) => structuredClone(current));
  const app = vi.fn(async (_command: AppCommand) => ({}));
  const client: AgentBridgeClient = {
    snapshot: async () => structuredClone(current),
    input,
    configure,
    bridge,
    app,
    subscribe: async () => () => undefined,
  };
  return { client, input, configure, bridge, app };
}

function mockSystemTheme(initialDark: boolean) {
  let matches = initialDark;
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const mediaQuery = {
    get matches() {
      return matches;
    },
    media: "(prefers-color-scheme: dark)",
    onchange: null,
    addEventListener: (
      _type: "change",
      listener: (event: MediaQueryListEvent) => void,
    ) => listeners.add(listener),
    removeEventListener: (
      _type: "change",
      listener: (event: MediaQueryListEvent) => void,
    ) => listeners.delete(listener),
  };
  vi.stubGlobal("matchMedia", vi.fn(() => mediaQuery));
  return (dark: boolean) => {
    matches = dark;
    act(() => {
      for (const listener of listeners) {
        listener({ matches: dark } as MediaQueryListEvent);
      }
    });
  };
}

test("renders six native Agent Keys, six command positions, and textual state", async () => {
  const { client } = fakeClient();
  render(<App client={client} />);

  await screen.findByRole("button", { name: /Agent 1: Task 1, thinking/ });
  expect(screen.getAllByRole("button", { name: /^Agent \d:/ })).toHaveLength(6);
  for (const slot of COMMAND_SLOTS) {
    expect(screen.getByRole("button", { name: new RegExp(`^${slot}:`) })).toBeEnabled();
  }
  expect(screen.getByText("THINKING")).toBeVisible();
  expect(screen.getByText("Fully compatible")).toBeVisible();
});

test.each([
  ["idle", "Idle"],
  ["recording", "Recording"],
  ["processing", "Processing"],
  ["ready", "Ready"],
  ["locked", "Locked"],
  ["stopping", "Stopping"],
  ["unknown", "Unknown"],
] as const)("microphone %s state is shown as explicit text", async (state, label) => {
  const value = snapshot();
  value.micState = state;
  const { client } = fakeClient(value);

  render(<App client={client} />);

  expect(await screen.findByText(label)).toBeVisible();
});

test("pointer cancel and lost capture each release a pressed native control exactly once", async () => {
  const first = fakeClient();
  const view = render(<App client={first.client} />);
  const mic = await screen.findByRole("button", { name: "ACT10_ACT11: Push to Talk" });

  fireEvent.pointerDown(mic, { button: 0, pointerId: 7 });
  fireEvent.pointerCancel(mic, { pointerId: 7 });
  await waitFor(() => expect(first.input).toHaveBeenCalledTimes(2));
  expect(first.input.mock.calls.map(([event]) => event.phase.type)).toEqual(["pressed", "released"]);

  view.unmount();
  const second = fakeClient();
  render(<App client={second.client} />);
  const agent = await screen.findByRole("button", { name: /Agent 1:/ });
  fireEvent.pointerDown(agent, { button: 0, pointerId: 8 });
  fireEvent.lostPointerCapture(agent, { pointerId: 8 });
  await waitFor(() => expect(second.input).toHaveBeenCalledTimes(2));
  expect(second.input.mock.calls.map(([event]) => event.phase.type)).toEqual(["pressed", "released"]);
});

test("semantic button activation sends one complete native phase pair", async () => {
  const semantic = fakeClient();
  const view = render(<App client={semantic.client} />);
  const agent = await screen.findByRole("button", { name: /Agent 1:/ });

  fireEvent.click(agent, { detail: 0 });

  await waitFor(() => expect(semantic.input).toHaveBeenCalledTimes(2));
  expect(semantic.input.mock.calls.map(([event]) => event.phase.type)).toEqual([
    "pressed",
    "released",
  ]);

  view.unmount();
  const pointer = fakeClient();
  render(<App client={pointer.client} />);
  const command = await screen.findByRole("button", { name: "ACT06: Fast Mode" });
  fireEvent.pointerDown(command, { button: 0, pointerId: 12 });
  fireEvent.pointerUp(command, { pointerId: 12 });
  fireEvent.click(command, { detail: 1 });

  await waitFor(() => expect(pointer.input).toHaveBeenCalledTimes(2));
  expect(pointer.input.mock.calls.map(([event]) => event.phase.type)).toEqual([
    "pressed",
    "released",
  ]);
});

test("encoder rotation and joystick movement preserve their native phase semantics", async () => {
  const { client, input } = fakeClient();
  render(<App client={client} />);

  fireEvent.click(await screen.findByRole("button", {
    name: "Rotate encoder counterclockwise",
  }));
  const joystickUp = screen.getByRole("button", {
    name: "Joystick up: composer.togglePlanMode",
  });
  fireEvent.pointerDown(joystickUp, { button: 0, pointerId: 9 });
  fireEvent.pointerUp(joystickUp, { pointerId: 9 });

  await waitFor(() => expect(input).toHaveBeenCalledTimes(3));
  expect(input.mock.calls.map(([event]) => ({
    control: event.control,
    phase: event.phase,
  }))).toEqual([
    {
      control: { type: "encoderRotate" },
      phase: { type: "rotated", steps: -1 },
    },
    {
      control: { type: "joystick", direction: "up" },
      phase: { type: "entered" },
    },
    {
      control: { type: "joystick", direction: "up" },
      phase: { type: "centered" },
    },
  ]);
});

test("appearance settings persist light, dark, and live system themes", async () => {
  const setSystemDark = mockSystemTheme(true);
  const { client, configure } = fakeClient();

  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  const shell = document.querySelector(".app-shell");
  expect(shell).toHaveAttribute("data-theme", "dark");
  expect(screen.queryByRole("radiogroup", { name: "Color theme" })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  const group = await screen.findByRole("radiogroup", { name: "Color theme" });
  expect(within(group).getByRole("radio", { name: "System" })).toBeChecked();

  fireEvent.click(within(group).getByRole("radio", { name: "Light" }));
  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setPreferences",
    themeMode: "light",
  }));
  await waitFor(() => expect(shell).toHaveAttribute("data-theme", "light"));

  setSystemDark(false);
  expect(shell).toHaveAttribute("data-theme", "light");

  fireEvent.click(within(group).getByRole("radio", { name: "System" }));
  await waitFor(() => expect(shell).toHaveAttribute("data-theme", "light"));
  setSystemDark(true);
  expect(shell).toHaveAttribute("data-theme", "dark");
});

test("Codex Micro import stays a preview until the user commits its revision", async () => {
  const { client, configure } = fakeClient();
  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", { name: "Preview import" }));

  expect(await screen.findByRole("dialog", { name: "Import preview" })).toBeVisible();
  expect(screen.getByText("Will change to FAST")).toBeVisible();
  expect(configure).toHaveBeenCalledWith({ type: "previewImport" });
  expect(configure).not.toHaveBeenCalledWith(expect.objectContaining({ type: "commitImport" }));

  fireEvent.click(screen.getByRole("button", { name: "Import and overwrite" }));
  await waitFor(() =>
    expect(configure).toHaveBeenCalledWith({ type: "commitImport", revision: "revision-1" })
  );
});

test("unsupported native capabilities disable only affected controls with text status", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "degraded",
    fullyCompatible: false,
  };
  value.capabilities.push(
    { id: "agent-slots", label: "Agent slots", status: "supported" },
    { id: "native-binding-override", label: "Native bindings", status: "supported" },
    { id: "joystick", label: "Joystick", status: "supported" },
    { id: "encoder", label: "Encoder", status: "supported" },
  );
  value.capabilities = value.capabilities.map((capability) =>
    capability.id === "keycap.APPR"
      ? { ...capability, status: "unsupported", reason: "Registry entry changed." }
      : capability
  );
  const { client } = fakeClient(value);
  render(<App client={client} />);

  expect(await screen.findByRole("button", { name: "ACT07: Approve" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Approve: Registry entry changed." })).toBeDisabled();
  expect(screen.getByRole("button", { name: "ACT06: Fast Mode" })).toBeEnabled();
  expect(screen.getAllByText("Connected with limited compatibility")).toHaveLength(2);
});

test("a missing native capability disables the affected control even if the summary is stale", async () => {
  const value = snapshot();
  value.capabilities = value.capabilities.filter(
    (capability) => capability.id !== "keycap.APPR",
  );
  const { client } = fakeClient(value);

  render(<App client={client} />);

  expect(await screen.findByRole("button", { name: "ACT07: Approve" })).toBeDisabled();
});

test("disconnected agents remain visible as disabled last-known state", async () => {
  const value = snapshot();
  value.bridge.state = "disconnected";
  value.bridge.fullyCompatible = false;
  value.agents[0].stale = true;
  const { client } = fakeClient(value);

  render(<App client={client} />);

  expect(await screen.findByRole("button", {
    name: "Agent 1: Task 1, thinking, last known",
  })).toBeDisabled();
  expect(screen.getByText("Last known · working")).toBeVisible();
});

test("single-key edits update immediately and remain one-step undoable", async () => {
  const { client, configure } = fakeClient();
  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", { name: "ACT06 Fast Mode" }));
  fireEvent.click(await screen.findByText("Terminal"));

  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setCommand",
    slot: "ACT06",
    binding: { type: "keycap", id: "TERM" },
  }));
  expect(await screen.findByRole("button", { name: "ACT06 Terminal" })).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Undo last change" }));
  await waitFor(() => expect(configure).toHaveBeenCalledWith({ type: "undo" }));
  expect(await screen.findByRole("button", { name: "ACT06 Fast Mode" })).toBeVisible();
});

test("joystick picker saves a Rust-compatible command binding", async () => {
  const { client, configure } = fakeClient();
  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", {
    name: "JOYSTICK UP composer.togglePlanMode",
  }));
  fireEvent.click(await screen.findByRole("button", { name: /Navigate back/ }));

  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setJoystick",
    direction: "up",
    binding: { type: "command", id: "navigateBack" },
  }));
  expect(await screen.findByRole("button", {
    name: "JOYSTICK UP navigateBack",
  })).toBeVisible();
  expect(screen.queryByText("Terminal")).not.toBeInTheDocument();

  fireEvent.change(screen.getByRole("combobox", { name: "Encoder mode" }), {
    target: { value: "reasoning" },
  });
  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setEncoderMode",
    mode: "reasoning",
  }));
});

test("joystick picker can reuse a skill discovered in the live layouts", async () => {
  const value = snapshot();
  value.importedLayout = structuredClone(value.layout);
  value.importedLayout.joystick.right = {
    type: "skill",
    identity: {
      name: "review-helper",
      path: "/skills/review-helper/SKILL.md",
    },
  };
  const { client, configure } = fakeClient(value);
  render(<App client={client} />);

  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", {
    name: "JOYSTICK UP composer.togglePlanMode",
  }));
  fireEvent.click(await screen.findByRole("button", { name: /review-helper/ }));

  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setJoystick",
    direction: "up",
    binding: {
      type: "skill",
      identity: {
        name: "review-helper",
        path: "/skills/review-helper/SKILL.md",
      },
    },
  }));
});

test("failed immediate configuration stays visible and retryable", async () => {
  const { client, configure } = fakeClient();
  configure.mockRejectedValueOnce(new Error("Unable to save the layout."));
  render(<App client={client} />);

  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", {
    name: "JOYSTICK UP composer.togglePlanMode",
  }));
  fireEvent.click(await screen.findByRole("button", { name: /Navigate back/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Unable to save the layout.",
  );
  expect(screen.getByRole("button", {
    name: "JOYSTICK UP composer.togglePlanMode",
  })).toBeVisible();
  expect(screen.getByRole("button", { name: /Navigate back/ })).toBeEnabled();
});

test("launch at login can be changed after onboarding", async () => {
  const { client, configure } = fakeClient();
  render(<App client={client} />);

  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  const launchAtLogin = await screen.findByRole("checkbox", {
    name: "Open Agent Bridge at login",
  });
  fireEvent.click(launchAtLogin);

  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "setPreferences",
    launchAtLogin: true,
  }));
  expect(launchAtLogin).toBeChecked();
});

test("onboarding reports a save failure and remains retryable", async () => {
  const value = snapshot();
  value.preferences.onboardingComplete = false;
  value.bridge.state = "restartRequired";
  const { client, configure } = fakeClient(value);
  configure.mockRejectedValueOnce(new Error("Unable to save preferences."));

  render(<App client={client} />);
  const continueButton = await screen.findByRole("button", {
    name: "Continue to controls",
  });
  fireEvent.click(continueButton);

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Unable to save preferences.",
  );
  expect(continueButton).toBeEnabled();
});

test("restart-required banner asks for explicit confirmation before touching ChatGPT", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "restartRequired",
    fullyCompatible: false,
    message: "Save any unsent Composer text before restarting.",
  };
  const { client, bridge } = fakeClient(value);

  render(<App client={client} />);
  fireEvent.click(await screen.findByRole("button", {
    name: "Restart ChatGPT and connect",
  }));

  expect(await screen.findByRole("dialog", { name: "Restart ChatGPT?" })).toBeVisible();
  expect(bridge).not.toHaveBeenCalled();
});

test("restart confirmation sends the typed confirmed command and closes after success", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "restartRequired",
    fullyCompatible: false,
  };
  const { client, bridge } = fakeClient(value);

  render(<App client={client} />);
  fireEvent.click(await screen.findByRole("button", {
    name: "Restart ChatGPT and connect",
  }));
  const dialog = await screen.findByRole("dialog", { name: "Restart ChatGPT?" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Restart and connect" }));

  await waitFor(() => expect(bridge).toHaveBeenCalledWith({
    type: "restart",
    confirmed: true,
  }));
  await waitFor(() => expect(screen.queryByRole("dialog", {
    name: "Restart ChatGPT?",
  })).not.toBeInTheDocument());
});

test("restart confirmation prevents duplicate clicks while the native restart is in progress", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "restartRequired",
    fullyCompatible: false,
  };
  const { client, bridge } = fakeClient(value);
  let finishRestart: ((next: AppSnapshot) => void) | undefined;
  bridge.mockImplementationOnce(() => new Promise((resolve) => {
    finishRestart = resolve;
  }));

  render(<App client={client} />);
  fireEvent.click(await screen.findByRole("button", {
    name: "Restart ChatGPT and connect",
  }));
  const dialog = await screen.findByRole("dialog", { name: "Restart ChatGPT?" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Restart and connect" }));

  expect(await within(dialog).findByRole("button", {
    name: "Restarting ChatGPT…",
  })).toBeDisabled();
  expect(bridge).toHaveBeenCalledTimes(1);

  finishRestart?.(value);
  await waitFor(() => expect(screen.queryByRole("dialog", {
    name: "Restart ChatGPT?",
  })).not.toBeInTheDocument());
});

test("a failed restart stays retryable and reports the native error", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "restartRequired",
    fullyCompatible: false,
  };
  const { client, bridge } = fakeClient(value);
  bridge.mockRejectedValueOnce(new Error("Unable to restart ChatGPT."));

  render(<App client={client} />);
  fireEvent.click(await screen.findByRole("button", {
    name: "Restart ChatGPT and connect",
  }));
  const dialog = await screen.findByRole("dialog", { name: "Restart ChatGPT?" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Restart and connect" }));

  expect(await screen.findByRole("button", {
    name: "Error: Unable to restart ChatGPT.",
  })).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Restart and connect" })).toBeEnabled();
  expect(bridge).toHaveBeenCalledTimes(1);
});

test("restore defaults asks for in-app confirmation before changing the layout", async () => {
  const { client, configure } = fakeClient();

  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", { name: "Restore defaults" }));

  expect(await screen.findByRole("dialog", {
    name: "Restore official defaults?",
  })).toBeVisible();
  expect(configure).not.toHaveBeenCalledWith(expect.objectContaining({
    type: "restoreDefaults",
  }));
});

test("confirmed restore sends the typed command and closes after success", async () => {
  const { client, configure } = fakeClient();

  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", { name: "Restore defaults" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Restore official defaults?",
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Restore defaults" }));

  await waitFor(() => expect(configure).toHaveBeenCalledWith({
    type: "restoreDefaults",
    confirmed: true,
  }));
  await waitFor(() => expect(screen.queryByRole("dialog", {
    name: "Restore official defaults?",
  })).not.toBeInTheDocument());
});

test("a failed restore remains open, reports the error, and can be retried", async () => {
  const { client, configure } = fakeClient();
  configure.mockRejectedValueOnce(new Error("Unable to restore the layout."));

  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Configure" }));
  fireEvent.click(await screen.findByRole("button", { name: "Restore defaults" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Restore official defaults?",
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Restore defaults" }));

  expect(await within(dialog).findByRole("alert")).toHaveTextContent(
    "Unable to restore the layout.",
  );
  expect(within(dialog).getByRole("button", { name: "Restore defaults" })).toBeEnabled();
  expect(configure).toHaveBeenCalledWith({
    type: "restoreDefaults",
    confirmed: true,
  });
});

test("diagnostics can be cleared and exported through typed app commands", async () => {
  const value = snapshot();
  value.diagnostics = [{
    at: "2026-07-30T00:00:00.000Z",
    category: "dispatch",
    outcome: "executed",
    durationMs: 12,
  }];
  const { client, app } = fakeClient(value);
  app.mockImplementation(async (command: AppCommand) => {
    if (command.type === "clearDiagnostics") {
      return { snapshot: { ...value, diagnostics: [] } };
    }
    if (command.type === "exportDiagnostics") {
      return { path: "/tmp/agent-bridge-diagnostics.json" };
    }
    return {};
  });

  render(<App client={client} />);
  await screen.findByText("Fully compatible");
  fireEvent.click(screen.getByRole("button", { name: "Diagnostics" }));
  expect(await screen.findByText("dispatch")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Clear" }));
  await waitFor(() => expect(screen.queryByText("dispatch")).not.toBeInTheDocument());
  expect(app).toHaveBeenCalledWith({ type: "clearDiagnostics" });

  fireEvent.click(screen.getByRole("button", { name: "Export" }));
  expect(await screen.findByText(
    "Exported to /tmp/agent-bridge-diagnostics.json",
  )).toBeVisible();
  expect(app).toHaveBeenCalledWith({ type: "exportDiagnostics" });
});

test("bridge banner re-detects a ready connection and reconnects a degraded one", async () => {
  const ready = fakeClient();
  const view = render(<App client={ready.client} />);
  fireEvent.click(await screen.findByRole("button", { name: "Re-detect" }));
  await waitFor(() => expect(ready.bridge).toHaveBeenCalledWith({ type: "probe" }));

  view.unmount();
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "degraded",
    fullyCompatible: false,
  };
  const degraded = fakeClient(value);
  render(<App client={degraded.client} />);
  fireEvent.click(await screen.findByRole("button", { name: "Reconnect" }));
  await waitFor(() => expect(degraded.bridge).toHaveBeenCalledWith({ type: "connect" }));

  fireEvent.click(screen.getByRole("button", { name: "Compatibility details" }));
  expect(await screen.findByRole("heading", {
    name: "Desktop compatibility",
  })).toBeVisible();
});

test("an incompatible bridge can re-run capability detection without restarting the app", async () => {
  const value = snapshot();
  value.bridge = {
    ...value.bridge,
    state: "incompatible",
    fullyCompatible: false,
    message: "The native capability probe failed.",
  };
  const { client, bridge } = fakeClient(value);

  render(<App client={client} />);
  fireEvent.click(await screen.findByRole("button", { name: "Re-detect" }));

  await waitFor(() => expect(bridge).toHaveBeenCalledWith({ type: "probe" }));
});

test("reset credit requires the full hold gesture before it is consumed", async () => {
  const value = snapshot();
  value.usage = {
    windows: [],
    resetCreditsAvailable: 1,
    resetCreditsApplicable: 1,
  };
  const { client, app } = fakeClient(value);
  render(<App client={client} />);
  const reset = await screen.findByRole("button", {
    name: "Hold 1.2s · 1 reset credit",
  });

  vi.useFakeTimers();
  try {
    fireEvent.pointerDown(reset, { button: 0, pointerId: 10 });
    act(() => vi.advanceTimersByTime(1199));
    expect(app).not.toHaveBeenCalled();

    act(() => vi.advanceTimersByTime(1));
    expect(app).toHaveBeenCalledWith({
      type: "consumeReset",
      confirmed: true,
    });
  } finally {
    vi.useRealTimers();
  }
});
