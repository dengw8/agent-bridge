#!/usr/bin/env node

import {
  chmod,
  copyFile,
  cp,
  mkdir,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectDirectory = resolve(scriptDirectory, "..");

function infoPlist() {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Agent Bridge</string>
  <key>CFBundleExecutable</key>
  <string>agent-bridge</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>com.dengwang.agent-bridge</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Agent Bridge</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
`;
}

export async function prepareMacOsDevBundle({
  executablePath,
  iconPath = join(projectDirectory, "src-tauri", "icons", "icon.icns"),
  resourcesPath = join(
    projectDirectory,
    "src-tauri",
    "resources",
    "renderer",
  ),
}) {
  const absoluteExecutable = resolve(executablePath);
  const appRoot = join(
    dirname(absoluteExecutable),
    "bundle",
    "macos",
    "Agent Bridge Dev.app",
  );
  const contentsDirectory = join(appRoot, "Contents");
  const macOsDirectory = join(contentsDirectory, "MacOS");
  const resourcesDirectory = join(contentsDirectory, "Resources");
  const bundledExecutable = join(macOsDirectory, "agent-bridge");
  const bundledRenderer = join(resourcesDirectory, "resources", "renderer");

  await mkdir(macOsDirectory, { recursive: true });
  await mkdir(resourcesDirectory, { recursive: true });
  await copyFile(absoluteExecutable, bundledExecutable);
  await chmod(bundledExecutable, (await stat(absoluteExecutable)).mode);
  await copyFile(iconPath, join(resourcesDirectory, "icon.icns"));
  await rm(bundledRenderer, { recursive: true, force: true });
  await cp(resourcesPath, bundledRenderer, { recursive: true });
  await writeFile(join(contentsDirectory, "Info.plist"), infoPlist());

  return bundledExecutable;
}

async function run() {
  if (process.platform !== "darwin") {
    throw new Error("The Agent Bridge app-bundle runner is macOS-only.");
  }
  const [executablePath, ...applicationArguments] = process.argv.slice(2);
  if (!executablePath) {
    throw new Error("Tauri did not provide the development executable path.");
  }

  const bundledExecutable = await prepareMacOsDevBundle({ executablePath });
  const child = spawn(bundledExecutable, applicationArguments, {
    env: process.env,
    stdio: "inherit",
  });
  const signals = ["SIGINT", "SIGTERM", "SIGHUP"];
  const signalHandlers = new Map(
    signals.map((signal) => [
      signal,
      () => {
        if (!child.killed) {
          child.kill(signal);
        }
      },
    ]),
  );
  for (const [signal, handler] of signalHandlers) {
    process.on(signal, handler);
  }

  const [code, signal] = await once(child, "exit");
  for (const [forwardedSignal, handler] of signalHandlers) {
    process.off(forwardedSignal, handler);
  }
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exitCode = code ?? 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  await run();
}
