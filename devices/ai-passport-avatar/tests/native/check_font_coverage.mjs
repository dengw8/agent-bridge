import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const fontRoot = resolve(root, "managed_components/lvgl__lvgl/src/font");

function glyphsIn(path) {
  const source = readFileSync(path, "utf8");
  return new Set(
    [...source.matchAll(/\/\* U\+([0-9A-F]{4,6})\b/g)].map((match) =>
      Number.parseInt(match[1], 16),
    ),
  );
}

function missingGlyphs(label, text, glyphs) {
  const missing = [...new Set([...text])]
    .map((character) => [character, character.codePointAt(0)])
    .filter(([, codePoint]) => !glyphs.has(codePoint));
  return missing.map(
    ([character, codePoint]) =>
      `${label}: ${character} (U+${codePoint.toString(16).toUpperCase()})`,
  );
}

const montserrat14 = glyphsIn(resolve(fontRoot, "lv_font_montserrat_14.c"));
const titleFont = new Set([
  ...glyphsIn(resolve(root, "main/lv_font_agent_title_16.c")),
  ...glyphsIn(resolve(fontRoot, "lv_font_source_han_sans_sc_16_cjk.c")),
]);
const uiSource = readFileSync(resolve(root, "main/avatar_ui.c"), "utf8");
const uiLiterals = [...uiSource.matchAll(/"([^"\\]*(?:\\.[^"\\]*)*)"/g)]
  .map((match) => match[1])
  .join("");

const missing = [
  ...missingGlyphs("Montserrat UI labels", uiLiterals, montserrat14),
  ...missingGlyphs(
    "session title from the reported device screen",
    "深入了解 ai-passport 项目 — 使用场景",
    titleFont,
  ),
];

assert.deepEqual(missing, [], `Missing UI glyphs:\n${missing.join("\n")}`);

console.log("AI Passport UI font coverage passed");
