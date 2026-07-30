(() => {
  const request = __REQUEST__;
  const isLayout = (value) => value && typeof value === "object" &&
    value.version === 1 && value.slots && typeof value.slots === "object" &&
    value.analogStick && typeof value.analogStick === "object" &&
    typeof value.encoderMode === "string";
  const sourceOf = (value) => {
    try { return typeof value === "function" ? Function.prototype.toString.call(value) : ""; }
    catch { return ""; }
  };
  const root = document.getElementById("root");
  const reactKey = root && Object.getOwnPropertyNames(root).find((key) => key.startsWith("__reactContainer$"));
  if (!root || !reactKey) throw new Error("Codex React root was not found.");

  const queue = [root[reactKey]];
  const seen = new Set();
  let bridgeFiber = null;
  let driverRow = null;
  let hidHandler = null;
  let joystickHandler = null;
  while (queue.length && seen.size < 30000) {
    const fiber = queue.pop();
    if (!fiber || seen.has(fiber)) continue;
    seen.add(fiber);
    const source = sourceOf(fiber.type);
    if (source.includes("codex-micro-hid-event") && source.includes("codex-micro-joystick-event")) {
      const rows = fiber.updateQueue?.memoCache?.data;
      if (Array.isArray(rows)) {
        for (const row of rows) {
          if (!Array.isArray(row)) continue;
          const functions = row.filter((value) => typeof value === "function");
          const hid = functions.filter((value) => {
            const candidate = sourceOf(value);
            return candidate.includes("push-to-talk-press") &&
              candidate.includes("encoder-press") &&
              candidate.includes("composer.decreaseReasoningEffort");
          });
          const joystick = functions.filter((value) => {
            const candidate = sourceOf(value);
            return candidate.includes("previousActiveDirection") && candidate.includes("activeDirection");
          });
          if (hid.length === 1 && joystick.length === 1) {
            bridgeFiber = fiber;
            driverRow = row;
            hidHandler = hid[0];
            joystickHandler = joystick[0];
            break;
          }
        }
      }
      if (bridgeFiber) break;
    }
    queue.push(fiber.child, fiber.sibling);
  }
  if (!bridgeFiber || !driverRow || !hidHandler || !joystickHandler) {
    throw new Error("Codex native binding phase handlers changed.");
  }

  const persistedLayouts = [];
  let hook = bridgeFiber.memoizedState;
  while (hook) {
    if (isLayout(hook.memoizedState)) persistedLayouts.push(hook.memoizedState);
    hook = hook.next;
  }
  if (!persistedLayouts.length) throw new Error("Codex persisted Micro layout hook was not found.");
  const counts = new Map();
  for (const value of driverRow) {
    if (isLayout(value)) counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  const normalized = [...counts]
    .filter(([layout, count]) =>
      count >= 2 &&
      !persistedLayouts.includes(layout) &&
      persistedLayouts.every((persisted) =>
        persisted.slots !== layout.slots && persisted.analogStick !== layout.analogStick
      )
    )
    .map(([layout]) => layout);
  if (normalized.length !== 1) throw new Error("A unique isolated native Micro layout clone was not found.");
  const layout = normalized[0];
  if (Object.isFrozen(layout) || Object.isFrozen(layout.slots) || Object.isFrozen(layout.analogStick)) {
    throw new Error("The isolated native Micro layout clone is immutable.");
  }

  if (request.kind === "probe") {
    const micStateRefs = [];
    let stateHook = bridgeFiber.memoizedState;
    while (stateHook) {
      const reference = stateHook.memoizedState;
      if (
        reference && typeof reference === "object" &&
        reference.current && typeof reference.current === "object" &&
        typeof reference.current.type === "string"
      ) {
        micStateRefs.push(reference);
      }
      stateHook = stateHook.next;
    }
    return {
      outcome: "supported",
      capability: "native-binding-override",
      nativeMicStateMachine: sourceOf(hidHandler).includes("push-to-talk-press"),
      nativeEncoderGesture: sourceOf(hidHandler).includes("encoder-press"),
      nativeJoystickHandler: true,
      persistedLayoutIsolated: true,
      nativeMicStateSignal: micStateRefs.length === 1,
    };
  }
  const toNativeBinding = (binding, previous) => {
    if (binding.type === "keycap") return { keycapId: binding.id };
    if (binding.type === "command") {
      return {
        keycapId: previous?.keycapId ?? "FAST",
        action: { type: "command", commandId: binding.id },
      };
    }
    return {
      keycapId: previous?.keycapId ?? "FAST",
      action: {
        type: "skill",
        skillName: binding.identity.name,
        skillPath: binding.identity.path,
      },
    };
  };

  if (request.kind === "binding") {
    const previous = layout.slots[request.slot];
    layout.slots[request.slot] = toNativeBinding(request.binding, previous);
    try {
      hidHandler({
        event: {
          key: request.slot === "ACT10_ACT11" ? "ACT10" : request.slot,
          act: request.pressed ? 1 : 0,
          slot: null,
          threadKey: null,
        },
      });
    } finally {
      layout.slots[request.slot] = previous;
    }
  } else if (request.kind === "joystick") {
    const previous = layout.analogStick[request.direction];
    layout.analogStick[request.direction] = request.binding.type === "command"
      ? { type: "command", commandId: request.binding.id }
      : {
          type: "skill",
          skillName: request.binding.identity.name,
          skillPath: request.binding.identity.path,
        };
    try {
      const angle = { up: 0.75, right: 0, down: 0.25, left: 0.5 };
      joystickHandler({ event: { angle: angle[request.direction], distance: request.active ? 1 : 0 } });
    } finally {
      layout.analogStick[request.direction] = previous;
    }
  } else if (request.kind === "encoder") {
    const previous = layout.encoderMode;
    layout.encoderMode = request.mode;
    try {
      const key = request.phase === "clockwise" ? "ENC_CW"
        : request.phase === "counterclockwise" ? "ENC_CC" : "ENC";
      const act = request.phase === "pressed" ? 1 : request.phase === "released" ? 0 : 2;
      const count = act === 2 ? Math.max(1, Math.min(32, Math.abs(request.steps ?? 1))) : 1;
      for (let index = 0; index < count; index += 1) {
        hidHandler({ event: { key, act, slot: null, threadKey: null } });
      }
    } finally {
      layout.encoderMode = previous;
    }
  } else {
    throw new Error("Unsupported binding request.");
  }
  return { outcome: "executed", capability: "native-binding-override" };
})()
