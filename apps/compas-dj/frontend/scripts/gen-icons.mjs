// Rasterizes src-tauri/app-icon.svg into the Tauri icon set.
// Run: `npm run icons` from frontend/.
import sharp from "sharp";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "src-tauri");
const source = readFileSync(join(root, "app-icon.svg"));
const out = join(root, "icons");

async function png(size) {
  return sharp(source, { density: 384 }).resize(size, size).png().toBuffer();
}

function ico(pngBuf) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(1, 4);
  const entry = Buffer.alloc(16);
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(pngBuf.length, 8);
  entry.writeUInt32LE(22, 12);
  return Buffer.concat([header, entry, pngBuf]);
}

const loose = {
  "icon.png": 512,
  "32x32.png": 32,
  "64x64.png": 64,
  "128x128.png": 128,
  "128x128@2x.png": 256,
};

for (const [name, size] of Object.entries(loose)) {
  writeFileSync(join(out, name), await png(size));
}
writeFileSync(join(out, "icon.ico"), ico(await png(256)));

const iconset = mkdtempSync(join(tmpdir(), "opendeck-iconset-"));
const named = {
  "icon_16x16.png": 16,
  "icon_16x16@2x.png": 32,
  "icon_32x32.png": 32,
  "icon_32x32@2x.png": 64,
  "icon_128x128.png": 128,
  "icon_128x128@2x.png": 256,
  "icon_256x256.png": 256,
  "icon_256x256@2x.png": 512,
  "icon_512x512.png": 512,
  "icon_512x512@2x.png": 1024,
};
const setDir = join(iconset, "icon.iconset");
execFileSync("mkdir", [setDir]);
for (const [name, size] of Object.entries(named)) {
  writeFileSync(join(setDir, name), await png(size));
}
execFileSync("iconutil", ["-c", "icns", setDir, "-o", join(out, "icon.icns")]);
rmSync(iconset, { recursive: true, force: true });
console.log(`icons written to ${out}`);
