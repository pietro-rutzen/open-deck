import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/mp3Mirror.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { isInsideMirror, mirrorDirectory, mp3CopyProgressLabel, selectedWatchFolder } = await import(
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

const music = "/Users/pietro/Music/party";
const nested = "/Users/pietro/Music/party/nights";
const other = "/Users/pietro/Music/other";
const folderCases = [
  { openPath: null, folders: [music], expected: null },
  { openPath: music, folders: [music, other], expected: music },
  { openPath: `${nested}/`, folders: [music], expected: music },
  { openPath: nested, folders: [music, nested], expected: null },
  { openPath: "/Users/pietro/Music/party-live", folders: [music], expected: null },
  { openPath: other, folders: [], expected: null },
];
for (const entry of folderCases) {
  assert.equal(selectedWatchFolder(entry.openPath, entry.folders), entry.expected);
}

const mirrorCases = [
  { openPath: music, root: music, directory: `${music}/mp3`, inside: false },
  { openPath: nested, root: music, directory: `${music}/mp3/nights`, inside: false },
  { openPath: `${music}/mp3`, root: music, directory: `${music}/mp3`, inside: true },
  { openPath: `${music}/mp3/nights`, root: music, directory: `${music}/mp3/nights`, inside: true },
];
for (const entry of mirrorCases) {
  assert.equal(mirrorDirectory(entry.openPath, entry.root), entry.directory);
  assert.equal(isInsideMirror(entry.openPath, [entry.root]), entry.inside);
}
assert.equal(isInsideMirror(`${music}/mp3`, [music, nested]), true);
assert.equal(isInsideMirror(nested, [music, nested]), false);
assert.equal(isInsideMirror(null, [music]), false);

const library = readFileSync(new URL("../src/components/Library.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const toolbarStart = library.indexOf('className="tl-toolbar"');
const toolbarEnd = library.indexOf('className="tl-sheet"', toolbarStart);
assert.ok(toolbarStart > 0 && toolbarEnd > toolbarStart);
const toolbar = library.slice(toolbarStart, toolbarEnd);
const folderList = library.slice(0, toolbarStart);
const controlOrder = [
  'aria-label="Search library"',
  "'+ Add tracks'",
  "'Listening mode'",
  "\n              Reset played\n",
  "\n              Reorder\n",
  "\n              MP3\n",
];
let cursor = -1;
for (const marker of controlOrder) {
  const at = toolbar.indexOf(marker);
  assert.ok(at > cursor, `toolbar order missing ${marker}`);
  cursor = at;
}
assert.equal(folderList.includes("Keep MP3 copies"), false);
assert.equal(folderList.includes("tl-mp3"), false);
assert.match(toolbar, /disabled=\{mp3Folder == null\}/);
assert.match(toolbar, /Open one watched folder to keep MP3 copies/);
assert.match(toolbar, /className=\{`add-btn\$\{mp3Folder && mp3On\[mp3Folder\] \? ' add-btn--on' : ''\}`\}/);
assert.match(toolbar, /Keep MP3 copies/);
assert.equal(toolbar.includes("tl-mp3"), false);
assert.equal(toolbar.includes("sq-check"), false);
assert.equal(css.includes("tl-mp3"), false);
assert.match(library, /name: 'mp3'/);
assert.match(library, /title=\{row\.kind === 'folder' && row\.mirror \? 'MP3 copies' : undefined\}/);
assert.match(library, /t\.source_path \|\| t\.path/);
assert.match(library, /listMp3Mirror/);
assert.equal(toolbar.includes("name: 'mp3'"), false);
assert.match(library, /MP3 copies follow the source folder reorder/);

console.log("MP3 mirror: toolbar control, folder selection, and mirror navigation passed");
