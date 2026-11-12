import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/mp3Mirror.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { isInsideMirror, mirrorDirectory, mp3ConversionPresentation, selectedWatchFolder } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`
);

const presentations = [
  {
    name: "finished hyperpop is not 0/2",
    state: { converted: 2, convertible: 2, alreadyMp3: 23, failures: [], active: false },
    fraction: "",
    title: /2 of 2 converted.*23 already MP3/,
  },
  {
    name: "partial sources stay visible after the worker idles",
    state: { converted: 3, convertible: 25, alreadyMp3: 0, failures: [], active: false },
    fraction: "3/25",
    title: /3 of 25 converted/,
  },
  {
    name: "a running folder shows converted over convertible",
    state: { converted: 3, convertible: 25, alreadyMp3: 0, failures: [], active: true },
    fraction: "3/25",
    title: /3 of 25 converted/,
  },
  {
    name: "a failure stays in the fraction and on the hover",
    state: {
      converted: 1,
      convertible: 2,
      alreadyMp3: 23,
      failures: ["one.flac: could not decode"],
      active: false,
    },
    fraction: "1/2",
    title: /Failed: one\.flac: could not decode/,
  },
  {
    name: "already-mp3 folders have nothing to paint",
    state: { converted: 0, convertible: 0, alreadyMp3: 10, failures: [], active: false },
    fraction: "",
    title: /10 already MP3/,
  },
];
for (const entry of presentations) {
  const shown = mp3ConversionPresentation(entry.state);
  assert.equal(shown.fraction, entry.fraction, entry.name);
  assert.match(shown.title, entry.title, entry.name);
  assert.equal(shown.title.includes("played"), false, entry.name);
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
  "'MP3'",
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
assert.match(toolbar, /mp3Presentation\.title/);
assert.match(source, /Keep MP3 copies/);
assert.equal(toolbar.includes("tl-mp3"), false);
assert.equal(toolbar.includes("sq-check"), false);
assert.equal(css.includes("tl-mp3"), false);
assert.match(library, /name: 'mp3'/);
assert.match(library, /title=\{row\.kind === 'folder' && row\.mirror \? 'MP3 copies' : undefined\}/);
assert.match(library, /t\.source_path \|\| t\.path/);
assert.match(library, /listMp3Mirror/);
assert.equal(toolbar.includes("name: 'mp3'"), false);
assert.match(library, /MP3 copies follow the source folder reorder/);
assert.match(library, /row\.kind === 'folder' && !row\.mirror/);
assert.match(library, /mp3ConversionStatus/);
assert.equal(library.includes("mp3ProgressLabel"), false);

console.log("MP3 mirror: toolbar control, folder selection, and mirror navigation passed");
