(async () => {
  const official = [
    "FAST", "APPR", "REJ", "SPLIT", "MIC", "CODEX", "BUG", "OAI", "TERM", "DWN",
    "DEL", "NEW", "NAV", "MAGIC", "DIFF", "PLAY", "GIT", "BRCH", "MRG", "PR",
    "PAINT", "LAB", "PARTY", "TIME", "MIND+", "MIND-", "SETUP", "FOLD", "UPL", "APPS",
  ];
  const urls = [...new Set([
    ...[...document.querySelectorAll("link[href], script[src]")].map((element) => element.href || element.src),
    ...performance.getEntriesByType("resource").map((entry) => entry.name),
  ])].filter((url) => url.includes("/assets/") && url.endsWith(".js"));
  const moduleUrl = (prefix) => urls.find((url) => url.includes(`/assets/${prefix}`));
  const layoutUrl = urls.find((url) => url.includes("/assets/codex-micro-layout-"));
  if (!layoutUrl) throw new Error("Codex Micro keycap registry is unavailable.");
  const layout = await import(layoutUrl);
  const getter = Object.values(layout).find((candidate) => {
    if (typeof candidate !== "function") return false;
    try { return candidate("FAST")?.id === "FAST"; } catch { return false; }
  });
  if (typeof getter !== "function") throw new Error("Codex Micro keycap registry changed.");

  let commandRunner = null;
  const commandsUrl = moduleUrl("run-command-");
  if (commandsUrl) {
    const commands = await import(commandsUrl);
    commandRunner = typeof commands.i === "function" ? commands.i : null;
  }
  const bridgeUrl = moduleUrl("codex-micro-bridge-");
  if (!commandRunner && bridgeUrl) {
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
        if (typeof namespace[parts[0]] === "function") commandRunner = namespace[parts[0]];
      }
      if (commandRunner) break;
    }
  }

  const keycaps = official.map((id) => {
    const entry = getter(id);
    const action = entry?.action;
    const available = entry?.id === id && (
      (action?.type === "command" && typeof commandRunner === "function") ||
      (id === "MIC" && action?.type === "named" && typeof action.label === "string") ||
      (action?.type === "external-url" && typeof action.url === "string") ||
      (action?.type === "composer-text" && typeof action.text === "string")
    );
    return { id, available };
  });
  return { keycaps };
})()
