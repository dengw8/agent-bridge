#!/usr/bin/env node

import { spawn } from "node:child_process";
import { once } from "node:events";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectDirectory = resolve(scriptDirectory, "..");

export function withMacOsDevRunnerEnvironment(
  arguments_,
  platform = process.platform,
  architecture = process.arch,
  environment = process.env,
  root = projectDirectory,
) {
  const result = { ...environment };
  if (platform !== "darwin" || arguments_[0] !== "dev") {
    return result;
  }
  const targetIndex = arguments_.findIndex(
    (argument) => argument === "--target" || argument === "-t",
  );
  const target =
    targetIndex >= 0
      ? arguments_[targetIndex + 1]
      : architecture === "arm64"
        ? "aarch64-apple-darwin"
        : "x86_64-apple-darwin";
  const runnerVariable = `CARGO_TARGET_${target
    .toUpperCase()
    .replaceAll("-", "_")}_RUNNER`;
  result[runnerVariable] = join(root, "scripts", "macos-dev-runner.mjs");
  return result;
}

async function run() {
  const cli = join(
    projectDirectory,
    "node_modules",
    "@tauri-apps",
    "cli",
    "tauri.js",
  );
  const child = spawn(
    process.execPath,
    [cli, ...process.argv.slice(2)],
    {
      cwd: projectDirectory,
      env: withMacOsDevRunnerEnvironment(process.argv.slice(2)),
      stdio: "inherit",
    },
  );
  const [code, signal] = await once(child, "exit");
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exitCode = code ?? 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  await run();
}
