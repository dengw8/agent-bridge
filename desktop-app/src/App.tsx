import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  COMMAND_SLOTS,
  DIRECTIONS,
  KEYCAPS,
  type AppSnapshot,
  type Binding,
  type CommandSlot,
  type ConfigResult,
  type Control,
  type Direction,
  type ImportPreviewItem,
  type InputPhase,
  type KeycapId,
} from "./model";
import {
  tauriClient,
  type AgentBridgeClient,
  type ConfigCommand,
} from "./tauri/client";

type Page = "control" | "configure" | "diagnostics";
type ConfigTarget =
  | { type: "command"; slot: CommandSlot }
  | { type: "joystick"; direction: Direction };

const GLYPHS: Record<KeycapId, string> = {
  FAST: "ϟ", APPR: "✓", REJ: "×", SPLIT: "↗", MIC: "◉", CODEX: "⌁",
  BUG: "!", OAI: "◎", TERM: ">_", DWN: "↓", DEL: "⌫", NEW: "+",
  NAV: "◇", MAGIC: "✦", DIFF: "Δ", PLAY: "▶", GIT: "◆", BRCH: "⑂",
  MRG: "⋈", PR: "↗", PAINT: "◌", LAB: "⌬", PARTY: "◔", TIME: "◷",
  "MIND+": "+", "MIND-": "−", SETUP: "⚙", FOLD: "▱", UPL: "↑", APPS: "✣",
};

const JOYSTICK_COMMANDS = [
  ["Plan mode", "composer.togglePlanMode"],
  ["Navigate forward", "navigateForward"],
  ["Toggle sidebar", "toggleSidebar"],
  ["Navigate back", "navigateBack"],
] as const;

const BRIDGE_COPY: Record<AppSnapshot["bridge"]["state"], string> = {
  chatGptNotRunning: "Waiting for ChatGPT Desktop",
  restartRequired: "Local Bridge needs a confirmed ChatGPT restart",
  connecting: "Connecting to the main Codex window",
  awaitingSnapshot: "Connected · waiting for native state",
  ready: "ChatGPT connected",
  degraded: "Connected with limited compatibility",
  incompatible: "This ChatGPT build is not compatible",
  disconnected: "ChatGPT closed or the Renderer changed",
  restarting: "Restarting ChatGPT with the local Bridge",
};

const MIC_STATE_LABELS: Record<NonNullable<AppSnapshot["micState"]>, string> = {
  idle: "Idle",
  recording: "Recording",
  processing: "Processing",
  ready: "Ready",
  locked: "Locked",
  stopping: "Stopping",
  unknown: "Unknown",
};

function bindingLabel(binding: Binding): string {
  if (binding.type === "keycap") {
    return KEYCAPS.find(([id]) => id === binding.id)?.[1] ?? binding.id;
  }
  if (binding.type === "command") return binding.id;
  if (binding.type === "skill") return `$${binding.identity.name}`;
  if (binding.type === "unsupported") return binding.label;
  return "None";
}

function keycapId(binding: Binding): KeycapId | undefined {
  return binding.type === "keycap" ? binding.id : undefined;
}

function Pressable({
  children,
  className,
  disabled,
  label,
  onEdge,
  downPhase = { type: "pressed" },
  upPhase = { type: "released" },
}: {
  children: ReactNode;
  className: string;
  disabled?: boolean;
  label: string;
  onEdge: (phase: InputPhase) => void;
  downPhase?: InputPhase;
  upPhase?: InputPhase;
}) {
  const pressed = useRef(false);
  const release = useCallback(() => {
    if (!pressed.current) return;
    pressed.current = false;
    onEdge(upPhase);
  }, [onEdge, upPhase]);
  const down = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (disabled || pressed.current || event.button !== 0) return;
    event.preventDefault();
    pressed.current = true;
    event.currentTarget.setPointerCapture(event.pointerId);
    onEdge(downPhase);
  };
  return (
    <button
      aria-label={label}
      className={className}
      disabled={disabled}
      onClick={(event) => {
        if (disabled || event.detail !== 0 || pressed.current) return;
        onEdge(downPhase);
        onEdge(upPhase);
      }}
      onContextMenu={(event) => event.preventDefault()}
      onLostPointerCapture={release}
      onPointerCancel={release}
      onPointerDown={down}
      onPointerUp={release}
      type="button"
    >
      {children}
    </button>
  );
}

function Onboarding({
  client,
  snapshot,
  onDone,
}: {
  client: AgentBridgeClient;
  snapshot: AppSnapshot;
  onDone: (snapshot: AppSnapshot) => void;
}) {
  const [launchAtLogin, setLaunchAtLogin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const finish = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const result = await client.configure({
        type: "setPreferences",
        launchAtLogin,
        onboardingComplete: true,
      });
      onDone(result.snapshot);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  };
  return (
    <main className="onboarding">
      <section>
        <p className="eyebrow">PERSONAL VALIDATION BUILD</p>
        <h1>Agent Bridge</h1>
        <p>
          Control the Codex UI already running in ChatGPT Desktop. The private
          Bridge uses a random loopback debugging port and never copies chat,
          terminal, Diff, microphone, or credential data into its logs.
        </p>
        <div className="risk-note">
          <strong>Local trust boundary</strong>
          <span>
            Other software running as your macOS user could attempt to reach
            ChatGPT&apos;s debugging port. Use this build only in a trusted local session.
          </span>
        </div>
        <label className="login-choice">
          <input
            checked={launchAtLogin}
            onChange={(event) => setLaunchAtLogin(event.currentTarget.checked)}
            type="checkbox"
          />
          <span>
            <strong>Open Agent Bridge at login</strong>
            <small>Recommended. This does not start a closed ChatGPT app.</small>
          </span>
        </label>
        <button className="primary" disabled={busy} onClick={finish} type="button">
          {busy ? "Saving…" : "Continue to controls"}
        </button>
        {error && <p className="onboarding-error" role="alert">{error}</p>}
        <small>Current state: {BRIDGE_COPY[snapshot.bridge.state]}</small>
      </section>
    </main>
  );
}

function BridgeBanner({
  snapshot,
  onBridge,
  onDiagnostics,
}: {
  snapshot: AppSnapshot;
  onBridge: (type: "connect" | "probe" | "restart") => void;
  onDiagnostics: () => void;
}) {
  const state = snapshot.bridge.state;
  return (
    <section className={`bridge-banner bridge-${state}`}>
      <span className="status-dot" />
      <div>
        <strong>{BRIDGE_COPY[state]}</strong>
        <small>{snapshot.bridge.message}</small>
      </div>
      <div className="banner-actions">
        {(state === "disconnected" || state === "degraded") && (
          <button onClick={() => onBridge("connect")} type="button">Reconnect</button>
        )}
        {state === "restartRequired" && (
          <button className="danger" onClick={() => onBridge("restart")} type="button">
            Restart ChatGPT and connect
          </button>
        )}
        {(state === "degraded" || state === "incompatible") && (
          <button onClick={onDiagnostics} type="button">Compatibility details</button>
        )}
        {(state === "ready" || state === "incompatible") && (
          <button onClick={() => onBridge("probe")} type="button">Re-detect</button>
        )}
      </div>
    </section>
  );
}

function ControlPanel({
  snapshot,
  send,
  onConfigure,
  client,
}: {
  snapshot: AppSnapshot;
  send: (control: Control, phase: InputPhase) => void;
  onConfigure: (target?: ConfigTarget) => void;
  client: AgentBridgeClient;
}) {
  const connected = snapshot.bridge.state === "ready" || snapshot.bridge.state === "degraded";
  const capabilityReady = (id: string) => {
    const capability = snapshot.capabilities.find((item) => item.id === id);
    return capability?.status === "supported";
  };
  const nativeBindingReady = capabilityReady("native-binding-override");
  const [resetHolding, setResetHolding] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const stopReset = () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = undefined;
    setResetHolding(false);
  };
  const startReset = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0 || event.currentTarget.disabled) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    setResetHolding(true);
    resetTimer.current = setTimeout(() => {
      setResetHolding(false);
      void client.app({ type: "consumeReset", confirmed: true });
    }, 1200);
  };
  const resetCount =
    snapshot.usage?.resetCreditsApplicable ??
    snapshot.usage?.resetCreditsAvailable ??
    0;
  const rotate = (steps: number) =>
    send({ type: "encoderRotate" }, { type: "rotated", steps });
  const onWheel = (event: WheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (connected && capabilityReady("encoder")) rotate(event.deltaY < 0 ? -1 : 1);
  };
  return (
    <div className="control-layout">
      <section className="device-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">CODEX MICRO · SOFTWARE EDITION</p>
            <h2>Your agents, at your fingertips.</h2>
          </div>
          <p className="device-status">
            Native source: <strong>{snapshot.agentSource}</strong> · Mic:{" "}
            <strong>{snapshot.micState ? MIC_STATE_LABELS[snapshot.micState] : "Unavailable"}</strong>
          </p>
        </div>
        <div className="micro-board">
          <span className="edge edge-top">↑</span>
          <span className="edge edge-left">Agent Bridge · OpenAI 2026</span>
          <span className="edge edge-right">Native Codex UI control</span>
          <span className="edge edge-bottom">Let&apos;s build</span>
          <div className="micro-grid">
            <div className="dial-module" onWheel={onWheel}>
              <button
                aria-label="Rotate encoder counterclockwise"
                className="dial-step dial-minus"
                disabled={!connected || !capabilityReady("encoder")}
                onClick={() => rotate(-1)}
                type="button"
              >−</button>
              <Pressable
                className="dial-knob"
                disabled={!connected || !capabilityReady("encoder")}
                label="Press reasoning encoder"
                onEdge={(phase) => send({ type: "encoderPress" }, phase)}
              >
                <i />
                <small>{snapshot.layout.encoderMode === "reasoning" ? "REASON" : "COMPOSE"}</small>
              </Pressable>
              <button
                aria-label="Rotate encoder clockwise"
                className="dial-step dial-plus"
                disabled={!connected || !capabilityReady("encoder")}
                onClick={() => rotate(1)}
                type="button"
              >+</button>
            </div>

            {snapshot.agents.map((agent) => (
              <Pressable
                className={`agent-key agent-${agent.slot + 1} state-${agent.status}${agent.selected ? " selected" : ""}`}
                disabled={!connected || !capabilityReady("agent-slots")}
                key={agent.slot}
                label={`Agent ${agent.slot + 1}: ${agent.title ?? "Not assigned"}, ${agent.status}${agent.stale ? ", last known" : ""}`}
                onEdge={(phase) => send({ type: "agent", slot: agent.slot }, phase)}
              >
                <span>{agent.slot + 1}</span>
                <em>{agent.status.toUpperCase()}</em>
                <strong>{agent.title || (agent.threadKey ? "New chat" : "Not assigned")}</strong>
                <small>
                  {agent.threadKey
                    ? `${agent.stale ? "Last known · " : ""}${agent.nativeStatus}`
                    : "Press to create a task"}
                </small>
                {agent.contextUsedPercent != null && (
                  <progress max="100" value={agent.contextUsedPercent} />
                )}
              </Pressable>
            ))}

            <div className="joystick" aria-label="Joystick">
              <i />
              {DIRECTIONS.map((direction) => (
                <Pressable
                  className={`stick-zone ${direction}`}
                  disabled={!connected || !capabilityReady("joystick") || !nativeBindingReady}
                  downPhase={{ type: "entered" }}
                  key={direction}
                  label={`Joystick ${direction}: ${bindingLabel(snapshot.layout.joystick[direction])}`}
                  onEdge={(phase) => send({ type: "joystick", direction }, phase)}
                  upPhase={{ type: "centered" }}
                >
                  {direction === "up" ? "↑" : direction === "right" ? "→" : direction === "down" ? "↓" : "←"}
                </Pressable>
              ))}
            </div>

            {COMMAND_SLOTS.map((slot) => {
              const binding = snapshot.layout.commandSlots[slot];
              const id = keycapId(binding);
              const bindingReady = nativeBindingReady &&
                (!id || capabilityReady(`keycap.${id}`));
              return (
                <Pressable
                  className={`command-key command-${slot.toLowerCase().replace(/_/g, "-")}${id === "MIC" ? " mic-key" : ""}`}
                  disabled={!connected || !bindingReady}
                  key={slot}
                  label={`${slot}: ${bindingLabel(binding)}`}
                  onEdge={(phase) => send({ type: "command", slot }, phase)}
                >
                  <span>{slot.replace("_ACT11", "/11")}</span>
                  <b>{id ? GLYPHS[id] : "·"}</b>
                  <strong>{bindingLabel(binding)}</strong>
                </Pressable>
              );
            })}

            <button
              className="touch-module"
              onClick={() => onConfigure()}
              type="button"
            >
              <i />
              <strong>CONFIG</strong>
              <small>Edit layout</small>
            </button>
          </div>
        </div>
      </section>

      <aside className="sidecar">
        <div className="sidecar-section sidecar-task">
          <p className="eyebrow">CURRENT TASK</p>
          <h2>{snapshot.activeThread?.title ?? "No active task"}</h2>
          <small>{snapshot.bridge.codexVersion ? `ChatGPT ${snapshot.bridge.codexVersion}` : "Version unavailable"}</small>
          <div className="legend">
            {["idle", "thinking", "complete", "input", "error"].map((status) => (
              <span key={status}><i className={`state-${status}`} />{status}</span>
            ))}
          </div>
        </div>
        <hr />
        <div className="sidecar-section sidecar-usage">
          <p className="eyebrow">ACCOUNT USAGE</p>
          <div className="usage-list">
            {(snapshot.usage?.windows ?? []).map((window) => (
              <div key={window.id}>
                <span>{window.kind === "five-hour" ? "5 hour" : window.kind}</span>
                <strong>{Math.round(window.remainingPercent)}% remaining</strong>
                <progress max="100" value={window.remainingPercent} />
              </div>
            ))}
            {!snapshot.usage?.windows.length && <small>Usage data unavailable</small>}
          </div>
          <button
            className={`reset-hold${resetHolding ? " holding" : ""}`}
            disabled={!connected || resetCount <= 0}
            onLostPointerCapture={stopReset}
            onPointerCancel={stopReset}
            onPointerDown={startReset}
            onPointerUp={stopReset}
            type="button"
          >
            <i />
            <span>{resetCount > 0 ? `Hold 1.2s · ${resetCount} reset credit${resetCount === 1 ? "" : "s"}` : "No applicable reset credit"}</span>
          </button>
        </div>
        <hr />
        <div className="sidecar-section sidecar-keycaps">
          <p className="eyebrow">OFFICIAL KEYCAPS</p>
          <div className="quick-keycaps">
            {KEYCAPS.map(([id, name]) => {
              const capability = snapshot.capabilities.find((item) => item.id === `keycap.${id}`);
              const disabled = !connected || capability?.status !== "supported";
              return (
                <Pressable
                  className="quick-keycap"
                  disabled={disabled}
                  key={id}
                  label={`${name}${capability?.reason ? `: ${capability.reason}` : ""}`}
                  onEdge={(phase) => send({ type: "keycap", id }, phase)}
                >
                  <span>{id}</span><strong>{GLYPHS[id]}</strong>
                </Pressable>
              );
            })}
          </div>
        </div>
      </aside>
    </div>
  );
}

function ConfigurePanel({
  snapshot,
  client,
  onSnapshot,
  initialTarget,
}: {
  snapshot: AppSnapshot;
  client: AgentBridgeClient;
  onSnapshot: (snapshot: AppSnapshot) => void;
  initialTarget?: ConfigTarget;
}) {
  const [target, setTarget] = useState<ConfigTarget | undefined>(initialTarget);
  const [preview, setPreview] = useState<ImportPreviewItem[]>();
  const [revision, setRevision] = useState<string>();
  const [query, setQuery] = useState("");
  const [restoreConfirmationOpen, setRestoreConfirmationOpen] = useState(false);
  const [restorePending, setRestorePending] = useState(false);
  const [restoreError, setRestoreError] = useState<string>();
  const [configError, setConfigError] = useState<string>();
  const apply = async (command: ConfigCommand) => {
    const result: ConfigResult = await client.configure(command);
    onSnapshot(result.snapshot);
    if (result.preview) setPreview(result.preview);
    if (result.importRevision) setRevision(result.importRevision);
  };
  const safeApply = async (command: ConfigCommand) => {
    setConfigError(undefined);
    try {
      await apply(command);
      return true;
    } catch (error) {
      setConfigError(error instanceof Error ? error.message : String(error));
      return false;
    }
  };
  const currentBinding = target?.type === "command"
    ? snapshot.layout.commandSlots[target.slot]
    : target?.type === "joystick"
      ? snapshot.layout.joystick[target.direction]
      : undefined;
  const choose = (binding: Binding) => {
    if (!target) return;
    void safeApply(target.type === "command"
      ? { type: "setCommand", slot: target.slot, binding }
      : { type: "setJoystick", direction: target.direction, binding });
  };
  const visibleKeycaps = KEYCAPS.filter(([id, name]) =>
    `${id} ${name}`.toLowerCase().includes(query.toLowerCase())
  );
  const visibleJoystickCommands = JOYSTICK_COMMANDS.filter(([label, id]) =>
    `${label} ${id}`.toLowerCase().includes(query.toLowerCase())
  );
  const visibleJoystickSkills = useMemo(() => {
    const skills = new Map<string, Extract<Binding, { type: "skill" }>>();
    for (const layout of [snapshot.layout, snapshot.importedLayout]) {
      if (!layout) continue;
      for (const binding of [
        ...Object.values(layout.commandSlots),
        ...Object.values(layout.joystick),
      ]) {
        if (binding.type === "skill") {
          skills.set(`${binding.identity.name}\0${binding.identity.path}`, binding);
        }
      }
    }
    return [...skills.values()].filter((binding) =>
      binding.identity.name.toLowerCase().includes(query.toLowerCase())
    );
  }, [query, snapshot.importedLayout, snapshot.layout]);
  const restoreDefaults = async () => {
    setRestorePending(true);
    setRestoreError(undefined);
    try {
      await apply({ type: "restoreDefaults", confirmed: true });
      setRestoreConfirmationOpen(false);
    } catch (error) {
      setRestoreError(String(error));
    } finally {
      setRestorePending(false);
    }
  };
  return (
    <div className="configure-layout">
      <section className="settings-card">
        <div className="section-heading">
          <div><p className="eyebrow">ONE LAYOUT</p><h2>Physical controls</h2></div>
          <button onClick={() => void safeApply({ type: "undo" })} type="button">Undo last change</button>
        </div>
        <div className="config-grid">
          {COMMAND_SLOTS.map((slot) => (
            <button
              className={target?.type === "command" && target.slot === slot ? "selected" : ""}
              key={slot}
              onClick={() => setTarget({ type: "command", slot })}
              type="button"
            >
              <small>{slot}</small>
              <strong>{bindingLabel(snapshot.layout.commandSlots[slot])}</strong>
            </button>
          ))}
          {DIRECTIONS.map((direction) => (
            <button
              className={target?.type === "joystick" && target.direction === direction ? "selected" : ""}
              key={direction}
              onClick={() => setTarget({ type: "joystick", direction })}
              type="button"
            >
              <small>JOYSTICK {direction.toUpperCase()}</small>
              <strong>{bindingLabel(snapshot.layout.joystick[direction])}</strong>
            </button>
          ))}
        </div>
        <div className="encoder-setting">
          <div><strong>Encoder mode</strong><small>Changes are saved immediately.</small></div>
          <select
            aria-label="Encoder mode"
            onChange={(event) => void safeApply({
              type: "setEncoderMode",
              mode: event.currentTarget.value as AppSnapshot["layout"]["encoderMode"],
            })}
            value={snapshot.layout.encoderMode}
          >
            <option value="composer-navigation">Composer Navigation</option>
            <option value="reasoning">Reasoning Only</option>
          </select>
        </div>
        <div className="import-actions">
          <div>
            <strong>Codex Micro import</strong>
            <small>Agent slots always stay live and are not overwritten.</small>
          </div>
          <button onClick={() => void safeApply({ type: "previewImport" })} type="button">
            Preview import
          </button>
          <button
            className="danger subtle"
            onClick={() => setRestoreConfirmationOpen(true)}
            type="button"
          >
            Restore defaults
          </button>
        </div>
        <label className="login-choice settings-login-choice">
          <input
            aria-label="Open Agent Bridge at login"
            checked={snapshot.preferences.launchAtLogin}
            onChange={(event) => void safeApply({
              type: "setPreferences",
              launchAtLogin: event.currentTarget.checked,
            })}
            type="checkbox"
          />
          <span>
            <strong>Open Agent Bridge at login</strong>
            <small>This never starts ChatGPT when ChatGPT is closed.</small>
          </span>
        </label>
        {configError && <p className="onboarding-error" role="alert">{configError}</p>}
      </section>

      <aside className="action-picker">
        <p className="eyebrow">{target ? "ACTION PICKER" : "SELECT A CONTROL"}</p>
        <h2>{target?.type === "command" ? target.slot : target?.direction ?? "Choose a key or direction"}</h2>
        {currentBinding && <p>Current: <strong>{bindingLabel(currentBinding)}</strong></p>}
        {target?.type === "command" && (
          <>
            <input
              aria-label="Search actions"
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search official keycaps"
              value={query}
            />
            <div className="action-list">
              {visibleKeycaps.map(([id, name]) => {
                const capability = snapshot.capabilities.find((item) => item.id === `keycap.${id}`);
                const unsupported = capability?.status !== "supported";
                return (
                  <button
                    disabled={unsupported}
                    key={id}
                    onClick={() => choose({ type: "keycap", id })}
                    title={capability?.reason}
                    type="button"
                  >
                    <span>{GLYPHS[id]}</span>
                    <div><strong>{name}</strong><small>{id} · {capability?.status ?? "not probed"}</small></div>
                  </button>
                );
              })}
            </div>
          </>
        )}
        {target?.type === "joystick" && (
          <>
            <input
              aria-label="Search actions"
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search joystick actions"
              value={query}
            />
            <div className="action-list">
              {visibleJoystickCommands.map(([label, id]) => (
                <button
                  key={id}
                  onClick={() => choose({ type: "command", id })}
                  type="button"
                >
                  <span>◇</span>
                  <div><strong>{label}</strong><small>{id} · command</small></div>
                </button>
              ))}
              {visibleJoystickSkills.map((binding) => (
                <button
                  key={`${binding.identity.name}\0${binding.identity.path}`}
                  onClick={() => choose(binding)}
                  type="button"
                >
                  <span>✣</span>
                  <div><strong>${binding.identity.name}</strong><small>Discovered skill</small></div>
                </button>
              ))}
              {"none".includes(query.toLowerCase()) && (
                <button onClick={() => choose({ type: "none" })} type="button">
                  <span>·</span>
                  <div><strong>None</strong><small>Disabled direction</small></div>
                </button>
              )}
            </div>
          </>
        )}
      </aside>

      {preview && (
        <div aria-label="Import preview" className="modal-backdrop" role="dialog">
          <section className="import-preview">
            <p className="eyebrow">IMPORT PREVIEW</p>
            <h2>Review every change</h2>
            <table>
              <thead><tr><th>Position</th><th>Agent Bridge</th><th>Codex Micro</th><th>Result</th></tr></thead>
              <tbody>
                {preview.map((item) => (
                  <tr key={item.position}>
                    <td>{item.position}</td><td>{item.current}</td><td>{item.imported}</td><td>{item.result}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div>
              <button onClick={() => { setPreview(undefined); setRevision(undefined); }} type="button">Cancel</button>
              <button
                className="primary"
                disabled={!revision}
                onClick={() => {
                  if (!revision) return;
                  void safeApply({ type: "commitImport", revision }).then((saved) => {
                    if (saved) {
                      setPreview(undefined);
                      setRevision(undefined);
                    }
                  });
                }}
                type="button"
              >
                Import and overwrite
              </button>
            </div>
          </section>
        </div>
      )}
      {restoreConfirmationOpen && (
        <div className="modal-backdrop">
          <section
            aria-labelledby="restore-confirmation-title"
            aria-modal="true"
            className="confirmation-dialog"
            role="dialog"
          >
            <p className="eyebrow">RESTORE LAYOUT</p>
            <h2 id="restore-confirmation-title">Restore official defaults?</h2>
            <p>
              This replaces the current command and joystick assignments immediately.
              The current layout preview is not retained.
            </p>
            {restoreError && <p className="confirmation-error" role="alert">{restoreError}</p>}
            <div className="confirmation-actions">
              <button
                disabled={restorePending}
                onClick={() => setRestoreConfirmationOpen(false)}
                type="button"
              >
                Cancel
              </button>
              <button
                className="danger"
                disabled={restorePending}
                onClick={() => void restoreDefaults()}
                type="button"
              >
                {restorePending ? "Restoring…" : "Restore defaults"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function DiagnosticsPanel({
  snapshot,
  client,
  onSnapshot,
}: {
  snapshot: AppSnapshot;
  client: AgentBridgeClient;
  onSnapshot: (snapshot: AppSnapshot) => void;
}) {
  const [exported, setExported] = useState<string>();
  const clear = async () => {
    const result = await client.app({ type: "clearDiagnostics" });
    if (result.snapshot) onSnapshot(result.snapshot);
  };
  const exportLogs = async () => {
    const result = await client.app({ type: "exportDiagnostics" });
    setExported(result.path);
  };
  return (
    <div className="diagnostics-layout">
      <section className="settings-card">
        <div className="section-heading">
          <div><p className="eyebrow">CAPABILITY MATRIX</p><h2>Desktop compatibility</h2></div>
          <span className={snapshot.bridge.fullyCompatible ? "compat-full" : "compat-partial"}>
            {snapshot.bridge.fullyCompatible ? "Fully compatible" : "Partial compatibility"}
          </span>
        </div>
        <div className="capability-list">
          {snapshot.capabilities.map((capability) => (
            <div key={capability.id}>
              <i className={`cap-${capability.status}`} />
              <span><strong>{capability.label}</strong><small>{capability.reason ?? capability.id}</small></span>
              <em>{capability.status}</em>
            </div>
          ))}
        </div>
      </section>
      <aside className="settings-card">
        <div className="section-heading">
          <div><p className="eyebrow">SANITIZED EVENTS</p><h2>Local diagnostics</h2></div>
          <div><button onClick={() => void clear()} type="button">Clear</button><button onClick={() => void exportLogs()} type="button">Export</button></div>
        </div>
        {exported && <p className="export-path">Exported to {exported}</p>}
        <ol className="diagnostic-log">
          {snapshot.diagnostics.map((entry, index) => (
            <li key={`${entry.at}-${index}`}>
              <time>{new Date(entry.at).toLocaleTimeString()}</time>
              <span>{entry.category}</span>
              <strong>{entry.outcome}</strong>
              <small>{entry.durationMs == null ? entry.code : `${entry.durationMs}ms`}</small>
            </li>
          ))}
        </ol>
      </aside>
    </div>
  );
}

export default function App({ client = tauriClient }: { client?: AgentBridgeClient }) {
  const [snapshot, setSnapshot] = useState<AppSnapshot>();
  const [page, setPage] = useState<Page>("control");
  const [configTarget, setConfigTarget] = useState<ConfigTarget>();
  const [toast, setToast] = useState<string>();
  const [restartConfirmationOpen, setRestartConfirmationOpen] = useState(false);
  const [restartPending, setRestartPending] = useState(false);
  const sequence = useRef(0);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void client.snapshot()
      .then((value) => { if (alive) setSnapshot(value); })
      .catch((error: unknown) => setToast(String(error)));
    void client.subscribe((value) => { if (alive) setSnapshot(value); })
      .then((next) => { unlisten = next; });
    return () => { alive = false; unlisten?.(); };
  }, [client]);

  useEffect(() => {
    if (client !== tauriClient) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    void listen("open_diagnostics", () => {
        if (alive) setPage("diagnostics");
      })
      .then((next) => { unlisten = next; });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [client]);

  const send = useCallback((control: Control, phase: InputPhase) => {
    const event = {
      source: "virtualUi" as const,
      control,
      phase,
      sequence: ++sequence.current,
      monotonicMs: performance.now(),
    };
    void client.input(event).catch((error: unknown) => setToast(String(error)));
  }, [client]);

  const bridge = async (type: "connect" | "probe" | "restart") => {
    if (type === "restart") setRestartPending(true);
    try {
      const next = await client.bridge(type === "restart"
        ? { type, confirmed: true }
        : { type });
      setSnapshot(next);
      if (type === "restart") setRestartConfirmationOpen(false);
    } catch (error) {
      setToast(String(error));
    } finally {
      if (type === "restart") setRestartPending(false);
    }
  };
  const configure = (target?: ConfigTarget) => {
    setConfigTarget(target);
    setPage("configure");
  };
  const nav = useMemo(() => [
    ["control", "Control"],
    ["configure", "Configure"],
    ["diagnostics", "Diagnostics"],
  ] as const, []);

  if (!snapshot) return <main className="loading"><span /><strong>Starting Agent Bridge…</strong></main>;
  if (!snapshot.preferences.onboardingComplete) {
    return <Onboarding client={client} onDone={setSnapshot} snapshot={snapshot} />;
  }

  return (
    <div className="app-shell" data-theme={snapshot.theme}>
      <header className="topbar">
        <div><p className="eyebrow">AGENT BRIDGE</p><h1>Codex Controls</h1></div>
        <nav aria-label="Primary navigation">
          {nav.map(([id, label]) => (
            <button
              aria-current={page === id ? "page" : undefined}
              key={id}
              onClick={() => setPage(id)}
              type="button"
            >{label}</button>
          ))}
        </nav>
        <span className={`top-status state-${snapshot.bridge.state}`}>
          {snapshot.bridge.fullyCompatible ? "Fully compatible" : BRIDGE_COPY[snapshot.bridge.state]}
        </span>
      </header>
      <main className="main-content">
        <BridgeBanner
          onBridge={(type) => {
            if (type === "restart") {
              setRestartConfirmationOpen(true);
            } else {
              void bridge(type);
            }
          }}
          onDiagnostics={() => setPage("diagnostics")}
          snapshot={snapshot}
        />
        {page === "control" && (
          <ControlPanel client={client} onConfigure={configure} send={send} snapshot={snapshot} />
        )}
        {page === "configure" && (
          <ConfigurePanel
            client={client}
            initialTarget={configTarget}
            onSnapshot={setSnapshot}
            snapshot={snapshot}
          />
        )}
        {page === "diagnostics" && (
          <DiagnosticsPanel client={client} onSnapshot={setSnapshot} snapshot={snapshot} />
        )}
      </main>
      {restartConfirmationOpen && (
        <div className="modal-backdrop">
          <section
            aria-labelledby="restart-confirmation-title"
            aria-modal="true"
            className="confirmation-dialog"
            role="dialog"
          >
            <p className="eyebrow">RESTART REQUIRED</p>
            <h2 id="restart-confirmation-title">Restart ChatGPT?</h2>
            <p>
              Save any unsent Composer text first. ChatGPT will close and reopen with
              the private local Bridge enabled.
            </p>
            <div className="confirmation-actions">
              <button
                disabled={restartPending}
                onClick={() => setRestartConfirmationOpen(false)}
                type="button"
              >
                Cancel
              </button>
              <button
                className="danger"
                disabled={restartPending}
                onClick={() => void bridge("restart")}
                type="button"
              >
                {restartPending ? "Restarting ChatGPT…" : "Restart and connect"}
              </button>
            </div>
          </section>
        </div>
      )}
      {toast && <button className="toast" onClick={() => setToast(undefined)} type="button">{toast}</button>}
    </div>
  );
}
