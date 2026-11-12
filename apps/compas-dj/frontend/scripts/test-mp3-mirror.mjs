import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/mp3Mirror.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { mp3CopyProgressLabel } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`
);

const cases = [
  { done: 0, total: 0, active: true, label: "" },
  { done: 3, total: 12, active: false, label: "" },
  { done: 3, total: 12, active: true, label: "3/12" },
  { done: 20, total: 12, active: true, label: "12/12" },
  { done: -1, total: 4, active: true, label: "0/4" },
  { done: 1.9, total: 4.2, active: true, label: "1/4" },
];
for (const entry of cases) {
  assert.equal(
    mp3CopyProgressLabel(entry.done, entry.total, entry.active),
    entry.label,
  );
}

const library = readFileSync(new URL("../src/components/Library.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
assert.match(library, /title="Keep MP3 copies"/);
assert.match(library, /aria-label="Keep MP3 copies"/);
assert.match(library, /className=\{`sq-check\$\{mp3On\[f\] \? ' sq-check--on' : ''\}`\}/);
assert.match(library, /src-mp3-progress/);
assert.match(css, /\.sq-check \{[^}]*border-radius:\s*0;/);
assert.match(css, /\.src-mp3-progress \{[^}]*width:\s*4\.2em;/);
assert.match(css, /\.src-mp3-progress \{[^}]*min-width:\s*4\.2em;/);

console.log("MP3 mirror: progress label, square checkbox, and fixed count slot passed");
