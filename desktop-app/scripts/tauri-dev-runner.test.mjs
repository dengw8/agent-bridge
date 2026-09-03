import { mkdtemp, mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { prepareMacOsDevBundle } from "./macos-dev-runner.mjs";
import { withMacOsDevRunnerEnvironment } from "./tauri.mjs";

const temporaryDirectories = [];

afterEach(async () => {
  const { rm } = await import("node:fs/promises");
  await Promise.all(
    temporaryDirectories.splice(0).map((path) =>
      rm(path, { recursive: true, force: true }),
    ),
  );
});

describe("macOS Tauri development runner", () => {
  it("routes npm run tauri dev through the app-bundle runner", () => {
    expect(
      withMacOsDevRunnerEnvironment(
        ["dev"],
        "darwin",
        "arm64",
        {},
        "/project",
      ),
    ).toEqual({
      CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER:
        "/project/scripts/macos-dev-runner.mjs",
    });
    expect(
      withMacOsDevRunnerEnvironment(
        ["build"],
        "darwin",
        "arm64",
        {},
        "/project",
      ),
    ).toEqual({});
    expect(
      withMacOsDevRunnerEnvironment(
        ["dev"],
        "linux",
        "arm64",
        {},
        "/project",
      ),
    ).toEqual({});
  });

  it("wraps the debug executable in a real macOS app bundle", async () => {
    const root = await mkdtemp(join(tmpdir(), "agent-bridge-runner-test-"));
    temporaryDirectories.push(root);
    const executablePath = join(root, "target", "debug", "agent-bridge");
    const iconPath = join(root, "icons", "icon.icns");
    const resourcesPath = join(root, "resources", "renderer");
    await mkdir(join(root, "target", "debug"), { recursive: true });
    await mkdir(join(root, "icons"), { recursive: true });
    await mkdir(resourcesPath, { recursive: true });
    await writeFile(executablePath, "debug executable", { mode: 0o755 });
    await writeFile(iconPath, "icon");
    await writeFile(join(resourcesPath, "bridge.js"), "bridge");

    const bundledExecutable = await prepareMacOsDevBundle({
      executablePath,
      iconPath,
      resourcesPath,
    });

    const appRoot = join(
      root,
      "target",
      "debug",
      "bundle",
      "macos",
      "Agent Bridge Dev.app",
    );
    expect(bundledExecutable).toBe(
      join(appRoot, "Contents", "MacOS", "agent-bridge"),
    );
    expect(await readFile(bundledExecutable, "utf8")).toBe("debug executable");
    expect((await stat(bundledExecutable)).mode & 0o111).not.toBe(0);
    expect(
      await readFile(join(appRoot, "Contents", "Resources", "icon.icns"), "utf8"),
    ).toBe("icon");
    expect(
      await readFile(
        join(
          appRoot,
          "Contents",
          "Resources",
          "resources",
          "renderer",
          "bridge.js",
        ),
        "utf8",
      ),
    ).toBe("bridge");

    const plist = await readFile(join(appRoot, "Contents", "Info.plist"), "utf8");
    expect(plist).toContain("<string>com.dengwang.agent-bridge</string>");
    expect(plist).toContain("<string>icon.icns</string>");
    expect(plist).toContain("<key>NSBluetoothAlwaysUsageDescription</key>");
  });
});
