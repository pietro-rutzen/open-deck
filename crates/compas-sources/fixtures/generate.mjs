// Synthesize a short original stereo tone and, on macOS, wrap it in the
// containers exercised by decode_formats.rs. The tone is project-generated PCM.
// afconvert is the system Audio File Convert tool; it is not installed by this script.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const directory = dirname(fileURLToPath(import.meta.url));
const sampleRate = 44100;
const frameCount = 8820;
const toneHz = 440;

const interleaved = new Int16Array(frameCount * 2);
for (let frame = 0; frame < frameCount; frame += 1) {
  const left = Math.sin((2 * Math.PI * toneHz * frame) / sampleRate) * 0.4;
  const right = Math.sin((2 * Math.PI * toneHz * frame) / sampleRate + 0.4) * 0.25;
  interleaved[frame * 2] = Math.round(left * 32767);
  interleaved[frame * 2 + 1] = Math.round(right * 32767);
}

const dataBytes = interleaved.length * 2;
const wav = Buffer.alloc(44 + dataBytes);
wav.write("RIFF", 0);
wav.writeUInt32LE(36 + dataBytes, 4);
wav.write("WAVE", 8);
wav.write("fmt ", 12);
wav.writeUInt32LE(16, 16);
wav.writeUInt16LE(1, 20);
wav.writeUInt16LE(2, 22);
wav.writeUInt32LE(sampleRate, 24);
wav.writeUInt32LE(sampleRate * 4, 28);
wav.writeUInt16LE(4, 32);
wav.writeUInt16LE(16, 34);
wav.write("data", 36);
wav.writeUInt32LE(dataBytes, 40);
for (let index = 0; index < interleaved.length; index += 1) {
  wav.writeInt16LE(interleaved[index], 44 + index * 2);
}

const mpeg4Adts = (mpeg2Adts) => {
  const mpeg4 = Buffer.from(mpeg2Adts);
  let offset = 0;
  while (offset + 7 <= mpeg4.length) {
    const sync = (mpeg4[offset] << 4) | (mpeg4[offset + 1] >> 4);
    if (sync !== 0xfff) {
      throw new Error(`ADTS sync missing at byte ${offset}`);
    }
    const frameLength =
      ((mpeg4[offset + 3] & 0x03) << 11) |
      (mpeg4[offset + 4] << 3) |
      (mpeg4[offset + 5] >> 5);
    if (frameLength < 7 || offset + frameLength > mpeg4.length) {
      throw new Error(`ADTS frame at byte ${offset} does not fit the file`);
    }
    mpeg4[offset + 1] &= 0xf7;
    offset += frameLength;
  }
  if (offset !== mpeg4.length) {
    throw new Error("ADTS frames did not cover the file");
  }
  return mpeg4;
};

mkdirSync(directory, { recursive: true });
const wavPath = join(directory, "tone.wav");
writeFileSync(wavPath, wav);

const conversions = [
  ["tone.aiff", "AIFF", "BEI16"],
  ["tone.aif", "AIFF", "BEI16"],
  ["tone.m4a", "m4af", "aac "],
  ["tone.aac", "adts", "aac "],
];

for (const [name, fileFormat, dataFormat] of conversions) {
  execFileSync(
    "afconvert",
    [
      wavPath,
      join(directory, name),
      "-f",
      fileFormat,
      "-d",
      `${dataFormat}@${sampleRate}`,
      "-c",
      "2",
      "-b",
      "64000",
    ],
    { stdio: "inherit" },
  );
}

// afconvert writes MPEG-2 ADTS (sync 0xFFF9). Symphonia 0.5.5 only syncs on
// MPEG-4 ADTS (0xFFF1). Keep the encoder output, then clear that one ID bit.
const adtsPath = join(directory, "tone.aac");
const mpeg2Adts = readFileSync(adtsPath);
writeFileSync(join(directory, "tone-mpeg2.aac"), mpeg2Adts);
writeFileSync(adtsPath, mpeg4Adts(mpeg2Adts));

console.log(
  `Wrote a ${frameCount}-frame ${sampleRate} Hz stereo tone and converted it with afconvert.`,
);
