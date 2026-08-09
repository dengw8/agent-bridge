import { readFile, readdir, mkdir, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.dirname(fileURLToPath(import.meta.url));
const checkOnly = process.argv.includes("--check");

function decodeHex(value, label) {
  if (!/^(?:[0-9a-f]{2})*$/i.test(value)) {
    throw new Error(`${label}: payloadHex must contain complete hexadecimal bytes`);
  }
  return Buffer.from(value, "hex");
}

function encodeFrame(packet, frameSize) {
  const payload = decodeHex(packet.payloadHex, packet.name);
  if (payload.length > 48) throw new Error(`${packet.name}: payload exceeds 48 bytes`);
  if (!Number.isInteger(packet.sequence) || packet.sequence < 1 || packet.sequence > 0xffffffff) {
    throw new Error(`${packet.name}: sequence must be a non-zero u32`);
  }
  const frame = Buffer.alloc(frameSize);
  frame[0] = 0xab;
  frame[1] = 1;
  frame[2] = 0;
  frame[3] = packet.typeId;
  frame[5] = payload.length;
  frame.writeUInt32LE(packet.sequence, 6);
  frame.writeUInt32LE(packet.senderUptimeMs, 10);
  payload.copy(frame, 14);
  return frame;
}

async function expectFile(file, expected) {
  let actual;
  try {
    actual = await readFile(file);
  } catch {
    throw new Error(`missing generated file: ${path.relative(root, file)}`);
  }
  if (!actual.equals(expected)) {
    throw new Error(`stale generated file: ${path.relative(root, file)}`);
  }
}

async function emit(file, contents) {
  if (checkOnly) return expectFile(file, contents);
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, contents);
}

const goldenManifest = JSON.parse(await readFile(path.join(root, "golden/manifest.json"), "utf8"));
if (goldenManifest.schemaVersion !== 1 || goldenManifest.protocolVersion !== "1.0" || goldenManifest.frameSize !== 63) {
  throw new Error("unsupported Golden Packet manifest version");
}
if (goldenManifest.packets.length !== 22) throw new Error("ABP v1.0 requires one packet for each of 22 message types");
const requiredTypeIds = [1, 2, 3, 4, 5, 6, 7, 8, 9, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76];
const actualTypeIds = goldenManifest.packets.map((packet) => packet.typeId).sort((left, right) => left - right);
if (JSON.stringify(actualTypeIds) !== JSON.stringify(requiredTypeIds)) {
  throw new Error("ABP v1.0 Golden corpus must contain every message type exactly once");
}

const names = new Set();
const frames = new Map();
for (const packet of goldenManifest.packets) {
  if (!/^[a-z0-9_]+$/.test(packet.name) || names.has(packet.name)) throw new Error(`invalid or duplicate packet name: ${packet.name}`);
  names.add(packet.name);
  const frame = encodeFrame(packet, goldenManifest.frameSize);
  frames.set(packet.name, frame);
  await emit(path.join(root, `golden/${packet.name}.bin`), frame);
}

const invalidManifest = JSON.parse(await readFile(path.join(root, "fuzz/manifest.json"), "utf8"));
if (invalidManifest.schemaVersion !== 1 || invalidManifest.protocolVersion !== "1.0") {
  throw new Error("unsupported invalid corpus manifest version");
}
const invalidatedTypes = [...new Set(invalidManifest.packets.map((packet) => packet.base))].sort();
const goldenNames = [...frames.keys()].sort();
if (JSON.stringify(invalidatedTypes) !== JSON.stringify(goldenNames)) {
  throw new Error("invalid corpus must derive at least one payload violation from every legal message");
}
const invalidOutputs = new Map();
for (const packet of invalidManifest.packets) {
  const base = frames.get(packet.base);
  if (!base) throw new Error(`${packet.name}: unknown base packet ${packet.base}`);
  const bytes = Buffer.from(base);
  for (const [offset, value] of packet.set ?? []) {
    if (!Number.isInteger(offset) || offset < 0 || offset >= bytes.length || !Number.isInteger(value) || value < 0 || value > 255) {
      throw new Error(`${packet.name}: invalid byte mutation`);
    }
    bytes[offset] = value;
  }
  for (const [start, end, value] of packet.fill ?? []) {
    if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end > bytes.length || start >= end ||
        !Number.isInteger(value) || value < 0 || value > 255) {
      throw new Error(`${packet.name}: invalid fill mutation`);
    }
    bytes.fill(value, start, end);
  }
  invalidOutputs.set(`${packet.name}.bin`, bytes);
}
const lengthBase = frames.get(invalidManifest.lengthCases.base);
const [firstLength, lastLength] = invalidManifest.lengthCases.truncationLengths;
for (let length = firstLength; length <= lastLength; length += 1) {
  invalidOutputs.set(`truncated_${String(length).padStart(2, "0")}.bin`, lengthBase.subarray(0, length));
}
const oversized = Buffer.alloc(invalidManifest.lengthCases.oversizedLength);
lengthBase.copy(oversized);
invalidOutputs.set(`oversized_${oversized.length}.bin`, oversized);

const corpusDirectory = path.join(root, "fuzz/corpus");
if (!checkOnly) {
  await rm(corpusDirectory, { recursive: true, force: true });
  await mkdir(corpusDirectory, { recursive: true });
}
for (const [name, bytes] of invalidOutputs) await emit(path.join(corpusDirectory, name), bytes);
if (checkOnly) {
  const actualGolden = (await readdir(path.join(root, "golden"))).filter((name) => name.endsWith(".bin")).sort();
  const expectedGolden = [...frames.keys()].map((name) => `${name}.bin`).sort();
  if (JSON.stringify(actualGolden) !== JSON.stringify(expectedGolden)) throw new Error("Golden corpus contains missing or unexpected files");
  const actual = (await readdir(corpusDirectory)).filter((name) => name.endsWith(".bin")).sort();
  const expected = [...invalidOutputs.keys()].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error("invalid corpus contains missing or unexpected files");
}

console.log(`${checkOnly ? "verified" : "generated"} ${frames.size} legal and ${invalidOutputs.size} invalid ABP v1.0 packets`);
