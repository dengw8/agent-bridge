(async () => {
  const request = __REQUEST__;
  const urls = [...new Set([
    ...[...document.querySelectorAll("link[href], script[src]")].map((element) => element.href || element.src),
    ...performance.getEntriesByType("resource").map((entry) => entry.name),
  ])].filter((url) => url.includes("/assets/") && url.endsWith(".js"));
  const moduleUrl = (prefix) => urls.find((url) => url.includes(`/assets/${prefix}`));

  if (request.kind === "agent") {
    if (!Number.isInteger(request.slot) || request.slot < 0 || request.slot > 5) {
      throw new Error("Invalid Agent slot.");
    }
    const namespaces = [];
    for (const url of urls) {
      try { namespaces.push(await import(url)); } catch {}
    }
    const bus = namespaces.flatMap((namespace) => Object.values(namespace)).find((candidate) =>
      candidate && typeof candidate === "object" &&
      (typeof candidate.dispatchHostMessage === "function" || typeof candidate.dispatchMessage === "function")
    );
    if (!bus) throw new Error("Codex native event bus was not found.");
    const dispatch = bus.dispatchHostMessage ?? bus.dispatchMessage;
    dispatch.call(bus, {
      type: "codex-micro-hid-event",
      event: {
        key: `AG0${request.slot}`,
        act: request.pressed ? 1 : 0,
        slot: request.slot,
        threadKey: request.threadKey ?? null,
      },
    });
    return { outcome: "executed", capability: "agent-slots" };
  }

  if (request.kind === "openThread") {
    const identity = (value) => {
      if (typeof value !== "string") return null;
      return value.match(/(?:^|:)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i)?.[1]?.toLowerCase() ?? value;
    };
    const target = identity(request.threadKey);
    const active = () => identity(
      document.querySelector("[data-above-composer-conversation-id]")?.getAttribute("data-above-composer-conversation-id")
      ?? document.querySelector('[data-app-action-sidebar-thread-id][aria-current="page"]')
        ?.getAttribute("data-app-action-sidebar-thread-id")
    );
    if (active() === target) return { outcome: "executed", capability: "agent-slots" };
    const item = [...document.querySelectorAll("[data-app-action-sidebar-thread-id]")]
      .find((element) => identity(element.getAttribute("data-app-action-sidebar-thread-id")) === target);
    if (!item) return { outcome: "unavailable", capability: "agent-slots" };
    const selector = 'button, a, [role="button"], [role="link"]';
    const clickable = item.matches(selector) ? item : item.querySelector(selector) ?? item.closest(selector) ?? item;
    if (typeof clickable.click === "function") clickable.click();
    else clickable.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
    const deadline = Date.now() + 1500;
    while (Date.now() < deadline) {
      if (active() === target) return { outcome: "executed", capability: "agent-slots" };
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    return { outcome: "unavailable", capability: "agent-slots" };
  }

  if (request.kind === "keycap") {
    const layoutUrl = moduleUrl("codex-micro-layout-");
    const commandsUrl = moduleUrl("run-command-");
    const bridgeUrl = moduleUrl("codex-micro-bridge-");
    const vscodeUrl = moduleUrl("vscode-api-");
    if (!layoutUrl) throw new Error("Codex Micro keycap registry is unavailable.");
    const layout = await import(layoutUrl);
    const getter = Object.values(layout).find((candidate) => {
      if (typeof candidate !== "function") return false;
      try { return candidate("FAST")?.id === "FAST"; } catch { return false; }
    });
    if (typeof getter !== "function") throw new Error("Codex Micro keycap registry changed.");
    const action = getter(request.id)?.action;
    if (!action) return { outcome: "unsupported", capability: `keycap.${request.id}` };
    if (action.type === "command") {
      let runner = null;
      if (commandsUrl) {
        const commands = await import(commandsUrl);
        runner = typeof commands.i === "function" ? commands.i : null;
      }
      if (!runner && bridgeUrl) {
        const source = await (await fetch(bridgeUrl)).text();
        const match = source.match(/([A-Za-z_$][\w$]*)\(\s*[A-Za-z_$][\w$]*\??\.command\s*,["'`]codex_micro_hid["'`]\)/);
        const local = match?.[1];
        const pattern = /import\s*\{([^}]*)\}\s*from\s*["']([^"']+)["']/g;
        let entry;
        while (local && (entry = pattern.exec(source))) {
          for (const specifier of entry[1].split(",")) {
            const parts = specifier.trim().split(/\s+as\s+/);
            if ((parts[1] ?? parts[0]) !== local) continue;
            const namespace = await import(new URL(entry[2], bridgeUrl).href);
            if (typeof namespace[parts[0]] === "function") runner = namespace[parts[0]];
          }
          if (runner) break;
        }
      }
      if (typeof runner !== "function") throw new Error("Codex command runner is unavailable.");
      return {
        outcome: runner(action.command, "codex_micro_hid") ? "executed" : "unavailable",
        capability: `keycap.${request.id}`,
      };
    }
    if (!vscodeUrl) throw new Error("Codex host bridge is unavailable.");
    const vscode = await import(vscodeUrl);
    const bus = [vscode.g, vscode.m, ...Object.values(vscode)].find((candidate) =>
      candidate && typeof candidate === "object" &&
      (typeof candidate.dispatchHostMessage === "function" || typeof candidate.dispatchMessage === "function")
    );
    if (!bus) throw new Error("Codex host event bus was not found.");
    if (action.type === "external-url" && typeof bus.dispatchMessage === "function") {
      bus.dispatchMessage("open-in-browser", {
        url: action.url,
        source: "manual",
        initiator: "open_in_browser_bridge",
      });
      return { outcome: "executed", capability: `keycap.${request.id}` };
    }
    if (action.type === "composer-text" && typeof bus.dispatchHostMessage === "function") {
      bus.dispatchHostMessage({ type: "codex-micro-insert-composer-text", text: action.text });
      return { outcome: "executed", capability: `keycap.${request.id}` };
    }
    return { outcome: "unsupported", capability: `keycap.${request.id}` };
  }

  if (request.kind === "consumeReset") {
    let client = null;
    for (const url of urls) {
      try {
        const namespace = await import(url);
        client = Object.values(namespace).find((candidate) =>
          candidate && typeof candidate === "object" &&
          typeof candidate.safeGet === "function" && typeof candidate.safePost === "function"
        );
        if (client) break;
      } catch {}
    }
    if (!client) throw new Error("Codex usage client is unavailable.");
    const details = await client.safeGet("/wham/rate-limit-reset-credits");
    const credit = Array.isArray(details?.credits)
      ? details.credits.find((candidate) => candidate?.status === "available" && candidate?.is_supported_by_plan !== false)
      : null;
    if (!credit?.id) return { outcome: "unavailable", capability: "usage-reset-credit" };
    const result = await client.safePost("/wham/rate-limit-reset-credits/consume", {
      requestBody: { credit_id: credit.id, redeem_request_id: request.requestId },
    });
    return {
      outcome: result?.code === "reset" || result?.code === "already_redeemed" ? "executed" : "failed",
      capability: "usage-reset-credit",
    };
  }
  throw new Error("Unsupported renderer action.");
})()
