// Build the engines.json manifest the app reads to find its downloadable engines.
//
// One manifest per release, next to the engine archives themselves, so the app
// has a single source of truth for "which file for my platform, how big, and
// what hash". Hardcoding those in offline.rs would mean every engine rebuild
// needs a code change and a release, and the hashes could silently drift from
// the uploaded bytes — this cannot drift, it is generated from the files.
//
// Usage:
//   node scripts/engines-manifest.mjs --dir <artifacts> --out engines.json \
//     --llama b11214 --sherpa 1.13.8
//
// Input files must be named smoothflow-<kind>-<target>.zip, e.g.
// smoothflow-stt-win-x64.zip, smoothflow-llama-macos-arm64.zip.

import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const TARGETS = new Set(["win-x64", "win-arm64", "linux-x64", "linux-arm64", "macos-arm64", "macos-x64"]);
const KINDS = new Set(["stt", "llama"]);

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key.startsWith("--")) throw new Error(`unexpected argument: ${key}`);
    out[key.slice(2)] = argv[i + 1];
  }
  for (const required of ["dir", "out", "llama", "sherpa"]) {
    if (!out[required]) throw new Error(`missing --${required}`);
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const pattern = /^smoothflow-(stt|llama)-(.+)\.zip$/;

const engines = [];
for (const file of readdirSync(args.dir).sort()) {
  const match = pattern.exec(file);
  if (!match) continue;
  const [, kind, target] = match;
  if (!KINDS.has(kind)) throw new Error(`${file}: unknown kind '${kind}'`);
  if (!TARGETS.has(target)) throw new Error(`${file}: unknown target '${target}'`);

  const path = join(args.dir, file);
  const bytes = readFileSync(path);
  engines.push({
    kind,
    target,
    file,
    size: statSync(path).size,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  });
}

// Every target must have both engines. A half-populated manifest would ship an
// app that downloads a runtime it cannot pair with a model.
const have = new Set(engines.map((engine) => `${engine.kind}:${engine.target}`));
const missing = [];
for (const target of TARGETS) {
  for (const kind of KINDS) {
    // Windows/Linux arm64 have no runner in the build matrix yet; only require
    // the targets we can actually produce.
    if (target.endsWith("arm64") && !target.startsWith("macos")) continue;
    if (!have.has(`${kind}:${target}`)) missing.push(`${kind}:${target}`);
  }
}
if (missing.length > 0) {
  console.error(`[engines-manifest] missing archives: ${missing.join(", ")}`);
  process.exit(1);
}

const manifest = {
  schema: 1,
  llama_build: args.llama,
  sherpa_onnx: args.sherpa,
  engines: engines.sort((a, b) => (a.kind + a.target).localeCompare(b.kind + b.target)),
};

writeFileSync(args.out, `${JSON.stringify(manifest, null, 2)}\n`);
const mb = (n) => (n / 1024 / 1024).toFixed(1);
for (const engine of manifest.engines) {
  console.log(`[engines-manifest] ${engine.kind.padEnd(5)} ${engine.target.padEnd(12)} ${mb(engine.size).padStart(6)} MB  ${engine.sha256.slice(0, 12)}`);
}
console.log(`[engines-manifest] wrote ${args.out} (${engines.length} engines)`);
