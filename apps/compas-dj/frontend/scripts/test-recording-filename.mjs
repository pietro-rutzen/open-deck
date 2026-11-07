import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/recordingFilename.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { recordingDefaultFilename } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`
);

const eveningUtc = new Date("2026-11-06T21:26:39.000Z");
const eveningName = recordingDefaultFilename(eveningUtc);
assert.equal(eveningName, "open-deck-mix-2026-11-06-21-26-39.wav");

const newYearUtc = new Date("2026-01-01T00:00:00.000Z");
const newYearName = recordingDefaultFilename(newYearUtc);
assert.equal(newYearName, "open-deck-mix-2026-01-01-00-00-00.wav");

for (const name of [eveningName, newYearName]) {
  assert.equal(name.startsWith("open-deck-mix-"), true);
  assert.equal(name.endsWith(".wav"), true);
  assert.equal(name.includes("compas-mix-"), false);
}

console.log("Recording default filename uses open-deck-mix and a UTC stamp.");
