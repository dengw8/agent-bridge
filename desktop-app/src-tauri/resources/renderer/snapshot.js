(async () => {
  const urls = [...new Set([
    ...[...document.querySelectorAll("link[href], script[src]")].map((element) => element.href || element.src),
    ...performance.getEntriesByType("resource").map((entry) => entry.name),
  ])].filter((url) => url.includes("/assets/") && url.endsWith(".js"));
  const slotSignalsUrl = urls.find((url) => url.includes("/assets/codex-micro-slot-signals-"));
  if (!slotSignalsUrl) throw new Error("Codex Micro slot signals are not loaded.");

  const namespaces = [];
  for (const url of urls) {
    try { namespaces.push(await import(url)); } catch {}
  }
  const values = namespaces.flatMap((namespace) => Object.values(namespace));
  const definitions = values.find((candidate) =>
    candidate && typeof candidate === "object" &&
    candidate.layout?.key === "codex-micro-layout" &&
    candidate.agentSource?.key === "codex-micro-agent-source"
  );
  if (!definitions) throw new Error("Codex Micro settings definitions were not found.");

  const bus = values.find((candidate) =>
    candidate && typeof candidate === "object" && candidate.handlers instanceof Map &&
    (typeof candidate.dispatchHostMessage === "function" || typeof candidate.dispatchMessage === "function")
  );
  if (!bus) throw new Error("Codex native event bus was not found.");
  const dispatch = bus.dispatchHostMessage ?? bus.dispatchMessage;
  if ((bus.handlers.get("codex-micro-hid-event")?.size ?? 0) === 0) {
    dispatch.call(bus, {
      type: "codex-micro-device-state-changed",
      state: { status: "connected", error: null, battery: { percentage: 100, isCharging: true } },
    });
  }

  const root = document.getElementById("root");
  const reactKey = root && Object.getOwnPropertyNames(root).find((key) => key.startsWith("__reactContainer$"));
  if (!root || !reactKey) throw new Error("Codex React root was not found.");
  const slotSignals = await import(slotSignalsUrl);
  const resolvers = Object.values(slotSignals).filter((candidate) =>
    candidate && typeof candidate === "object" &&
    typeof candidate.resolve === "function" &&
    typeof candidate.createSubscriberAtom === "function"
  );
  if (!resolvers.length) throw new Error("Codex Micro slot resolver was not found.");

  const queue = [root[reactKey]];
  const seen = new Set();
  const queryClients = new Set();
  let found = null;
  while (queue.length && seen.size < 30000 && !found) {
    const fiber = queue.pop();
    if (!fiber || seen.has(fiber)) continue;
    seen.add(fiber);
    const contextValues = [fiber.memoizedProps?.value];
    let dependency = fiber.dependencies?.firstContext;
    while (dependency) {
      contextValues.push(dependency.memoizedValue);
      dependency = dependency.next;
    }
    const maps = contextValues.filter((value) => value instanceof Map);
    for (const value of contextValues) {
      if (value && typeof value.getQueryCache === "function" && typeof value.getQueryData === "function") {
        queryClients.add(value);
      }
    }
    for (const chain of maps) {
      for (const node of chain.values()) {
        if (!node?.store || typeof node.store.get !== "function") continue;
        for (const resolver of resolvers) {
          try {
            const slots = node.store.get(resolver.resolve(node, chain));
            if (Array.isArray(slots) && slots.length === 6 && slots.every((slot, index) => slot?.id === index)) {
              found = { node, slots };
              break;
            }
          } catch {}
        }
        if (found) break;
      }
      if (found) break;
    }
    queue.push(fiber.child, fiber.sibling);
  }
  if (!found) throw new Error("Codex Micro slot store was not found.");

  const sourceOf = (value) => {
    try { return typeof value === "function" ? Function.prototype.toString.call(value) : ""; }
    catch { return ""; }
  };
  const bridgeQueue = [root[reactKey]];
  const bridgeSeen = new Set();
  let nativeMicState = null;
  while (bridgeQueue.length && bridgeSeen.size < 30000 && nativeMicState == null) {
    const fiber = bridgeQueue.pop();
    if (!fiber || bridgeSeen.has(fiber)) continue;
    bridgeSeen.add(fiber);
    const source = sourceOf(fiber.type);
    if (source.includes("codex-micro-hid-event") && source.includes("codex-micro-joystick-event")) {
      const rows = fiber.updateQueue?.memoCache?.data;
      const ownsNativeHandler = Array.isArray(rows) && rows.some((row) =>
        Array.isArray(row) && row.some((value) => {
          const candidate = sourceOf(value);
          return candidate.includes("push-to-talk-press") &&
            candidate.includes("encoder-press") &&
            candidate.includes("composer.decreaseReasoningEffort");
        })
      );
      if (ownsNativeHandler) {
        const stateRefs = [];
        let hook = fiber.memoizedState;
        while (hook) {
          const reference = hook.memoizedState;
          if (
            reference && typeof reference === "object" &&
            reference.current && typeof reference.current === "object" &&
            typeof reference.current.type === "string"
          ) {
            stateRefs.push(reference.current.type);
          }
          hook = hook.next;
        }
        if (stateRefs.length === 1) {
          const state = stateRefs[0].toLowerCase();
          nativeMicState = state === "idle" ? "idle"
            : state.includes("lock") ? "locked"
              : state.includes("stop") || state.includes("suppress") ? "stopping"
                : state.includes("process") || state.includes("transcrib") ? "processing"
                  : state.includes("ready") ? "ready"
                    : state.includes("record") || state.includes("listen") ? "recording"
                      : "unknown";
        }
      }
    }
    bridgeQueue.push(fiber.child, fiber.sibling);
  }

  let nativeLayout = definitions.layout.default;
  let agentSource = definitions.agentSource.default;
  const directReader = values.find((candidate) => {
    if (typeof candidate !== "function" || candidate.length !== 1) return false;
    const source = Function.prototype.toString.call(candidate);
    return source.includes("get-setting") && source.includes(".default");
  });
  if (directReader) {
    try {
      const layout = await directReader(definitions.layout);
      const source = await directReader(definitions.agentSource);
      if (layout?.version === 1 && ["pinned", "recent", "priority", "custom"].includes(source)) {
        nativeLayout = layout;
        agentSource = source;
      }
    } catch {}
  }
  if (nativeLayout === definitions.layout.default) {
    const readers = values.filter((candidate) => {
      if (typeof candidate !== "function" || candidate.length !== 2) return false;
      const source = Function.prototype.toString.call(candidate);
      return source.includes(".key") && source.includes(".default");
    });
    for (const reader of readers) {
      try {
        const get = found.node.store.get.bind(found.node.store);
        const layout = await reader(get, definitions.layout);
        const source = await reader(get, definitions.agentSource);
        if (layout?.version === 1 && ["pinned", "recent", "priority", "custom"].includes(source)) {
          nativeLayout = layout;
          agentSource = source;
          break;
        }
      } catch {}
    }
  }

  const officialKeycaps = new Set([
    "FAST", "APPR", "REJ", "SPLIT", "MIC", "CODEX", "BUG", "OAI", "TERM", "DWN",
    "DEL", "NEW", "NAV", "MAGIC", "DIFF", "PLAY", "GIT", "BRCH", "MRG", "PR",
    "PAINT", "LAB", "PARTY", "TIME", "MIND+", "MIND-", "SETUP", "FOLD", "UPL", "APPS",
  ]);
  const asBinding = (binding, allowKeycap) => {
    if (!binding || typeof binding !== "object") return { type: "none" };
    const action = binding.action ?? binding;
    if (action.type === "command" && typeof (action.commandId ?? action.command) === "string") {
      return { type: "command", id: action.commandId ?? action.command };
    }
    if (action.type === "skill" && typeof action.skillName === "string" && typeof action.skillPath === "string") {
      return { type: "skill", identity: { name: action.skillName, path: action.skillPath } };
    }
    if (allowKeycap && typeof binding.keycapId === "string") {
      return officialKeycaps.has(binding.keycapId)
        ? { type: "keycap", id: binding.keycapId }
        : { type: "unsupported", label: `Unknown keycap ${binding.keycapId}` };
    }
    return { type: "none" };
  };
  const commandSlots = {};
  for (const id of ["ACT06", "ACT07", "ACT08", "ACT09", "ACT10_ACT11", "ACT12"]) {
    commandSlots[id] = asBinding(nativeLayout.slots?.[id], true);
  }
  const joystick = {};
  for (const direction of ["up", "right", "down", "left"]) {
    joystick[direction] = asBinding(nativeLayout.analogStick?.[direction], false);
  }
  const importedLayout = {
    commandSlots,
    joystick,
    encoderMode: nativeLayout.encoderMode === "reasoning" ? "reasoning" : "composer-navigation",
  };

  let usage;
  for (const client of queryClients) {
    try {
      const query = client.getQueryCache().getAll().find((candidate) =>
        JSON.stringify(candidate.queryKey) === '["rate-limit-status"]'
      );
      const data = query?.state?.data;
      const rateLimit = data?.rate_limit;
      if (!rateLimit) continue;
      const normalize = (window, role) => {
        const used = Number(window?.used_percent);
        if (!Number.isFinite(used)) return null;
        const seconds = Number(window?.limit_window_seconds);
        const minutes = Number.isFinite(seconds) ? seconds / 60 : null;
        const kind = minutes != null && Math.abs(minutes - 300) <= 1 ? "five-hour"
          : minutes != null && Math.abs(minutes - 10080) <= 1 ? "weekly" : "other";
        const reset = typeof window.reset_at === "number"
          ? (window.reset_at < 100000000000 ? window.reset_at * 1000 : window.reset_at)
          : Date.parse(window.reset_at);
        return {
          id: kind === "other" ? `${role}-${minutes ?? "unknown"}` : kind,
          kind,
          remainingPercent: 100 - Math.max(0, Math.min(100, used)),
          resetsAt: Number.isFinite(reset) ? reset : null,
        };
      };
      const windows = [
        normalize(rateLimit.primary_window, "primary"),
        normalize(rateLimit.secondary_window, "secondary"),
      ].filter(Boolean);
      const available = Number(data.rate_limit_reset_credits?.available_count);
      const applicable = Number(data.rate_limit_reset_credits?.applicable_available_count);
      usage = {
        windows,
        resetCreditsAvailable: Number.isFinite(available) ? Math.max(0, Math.floor(available)) : null,
        resetCreditsApplicable: Number.isFinite(applicable) ? Math.max(0, Math.floor(applicable)) : null,
      };
      break;
    } catch {}
  }

  const activeElement = document.querySelector(
    '[data-app-action-sidebar-thread-id][data-app-action-sidebar-thread-active="true"], [data-app-action-sidebar-thread-id][aria-current="page"]'
  );
  const activeThreadKey = document.querySelector("[data-above-composer-conversation-id]")
    ?.getAttribute("data-above-composer-conversation-id")
    ?? activeElement?.getAttribute("data-app-action-sidebar-thread-id")
    ?? null;
  const activeThreadTitle = activeElement
    ? (activeElement.getAttribute("aria-label") ?? activeElement.textContent ?? "").trim().slice(0, 240) || null
    : null;
  const themeWords = [
    document.documentElement.dataset.theme,
    document.documentElement.className,
    document.body?.dataset?.theme,
    document.body?.className,
    getComputedStyle(document.documentElement).colorScheme,
  ].filter(Boolean).join(" ").toLowerCase();
  const theme = /(^|[\s_-])light($|[\s_-])/.test(themeWords) ? "light" : "dark";

  return {
    agents: found.slots.map((slot, index) => ({
      slot: index,
      threadKey: typeof slot.threadKey === "string" ? slot.threadKey : null,
      title: typeof slot.title === "string" ? slot.title.slice(0, 240) : null,
      nativeStatus: typeof slot.status === "string" ? slot.status : "idle",
      selected: Boolean(slot.selected),
      contextUsedPercent: Number.isFinite(slot.contextUsedPercent) ? slot.contextUsedPercent : null,
    })),
    agentSource,
    activeThread: activeThreadKey ? { threadKey: activeThreadKey, title: activeThreadTitle } : null,
    importedLayout,
    theme,
    usage: usage ?? null,
    micState: nativeMicState,
  };
})()
