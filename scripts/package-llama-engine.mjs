// Trim an extracted llama.cpp release into the runtime SmoothFlow actually uses.
//
// SmoothFlow only ever runs `llama-server`. The upstream archive ships ~50 files:
// a dozen CLI tools, their impl libraries, the RPC backend, and 14 per-CPU-tier
// ggml-cpu variants. Everything except the server and the one CPU tier that
// matches the machine is dead weight in an installer.
//
// The keep list is not a guess: it is the hard-import closure of llama-server.exe
// (verified by launching the trimmed tree and loading a real GGUF). The per-tier
// ggml-cpu-* files are dlopen'd, not imported, so dropping variants can never
// cause a missing-DLL crash — it only forfeits AVX-512 on CPUs we do not ship a
// tier for, which ggml-cpu-zen4 covers.
//
// Usage:
//   node scripts/package-llama-engine.mjs --from <extracted-dir> --to <out-dir> --target win-x64
//
// Prints the kept files and total size. Exits non-zero if llama-server or a
// required library is missing, so a broken upstream archive fails CI loudly
// instead of shipping an installer whose engine cannot start.

import { copyFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { basename, join } from "node:path";

// Families that must survive. Matching is on the base name with every library
// suffix (`.dll`, `.so`, `.so.0`, `.0.25.3.dylib`, ...) stripped, so versioned
// sonames and their symlinks are kept together.
const KEEP = new Set([
  "llama-server",
  "llama-server-impl",
  "llama-common",
  "llama",
  "mtmd",
  "ggml",
  "ggml-base",
  "ggml-cpu",
  "ggml-metal",
  "ggml-blas",
  "omp",
]);

// One CPU tier beyond the portable baseline. `ggml-cpu-x64` (SSE4.2) runs
// everywhere; these two cover AVX2 and AVX-512 without shipping the other 11.
const KEEP_CPU_TIERS = new Set(["ggml-cpu-x64", "ggml-cpu-haswell", "ggml-cpu-zen4"]);

// Hard requirement: llama-server refuses to start without these.
const REQUIRED = ["llama-server", "llama-server-impl", "llama-common", "llama", "mtmd", "ggml", "ggml-base"];

function baseName(file) {
  let name = basename(file);
  if (name.startsWith("lib")) name = name.slice(3);
  // .dll | .so | .so.0 | .so.0.5.0 | .0.dylib | .0.5.0.dylib | .dylib
  name = name.replace(/\.(dll|dylib|exe)$/i, "");
  name = name.replace(/\.\d+(\.\d+)*$/, "");
  name = name.replace(/\.so$/i, "");
  return name.toLowerCase();
}

// Upstream ships both a shared library and a same-named executable family
// (`libllama.so` vs the `llama` CLI dispatcher). Only llama-server is ever run,
// so executables are matched by exact name and libraries by family.
function isLibrary(file) {
  return /\.(dll|dylib)$/i.test(file) || /\.so(\.\d+)*$/i.test(file);
}

function shouldKeep(file) {
  const base = baseName(file);
  if (base.startsWith("license")) return true;
  if (!isLibrary(file)) return base === "llama-server";
  if (KEEP.has(base)) return true;
  return KEEP_CPU_TIERS.has(base);
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key.startsWith("--")) throw new Error(`unexpected argument: ${key}`);
    out[key.slice(2)] = argv[i + 1];
  }
  for (const required of ["from", "to", "target"]) {
    if (!out[required]) throw new Error(`missing --${required}`);
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const files = readdirSync(args.from, { withFileTypes: true })
  .filter((entry) => entry.isFile())
  .map((entry) => entry.name)
  .sort();

const kept = files.filter(shouldKeep);
const dropped = files.filter((file) => !shouldKeep(file));

const keptBases = new Set(kept.map(baseName));
const missing = REQUIRED.filter((required) => !keptBases.has(required));
if (missing.length > 0) {
  console.error(`[package-llama] ${args.target}: upstream archive is missing ${missing.join(", ")}`);
  console.error(`[package-llama] kept ${kept.length} of ${files.length} files`);
  process.exit(1);
}

mkdirSync(args.to, { recursive: true });
let bytes = 0;
for (const file of kept) {
  copyFileSync(join(args.from, file), join(args.to, file));
  bytes += statSync(join(args.from, file)).size;
}

const mb = (n) => (n / 1024 / 1024).toFixed(1);
const before = files.reduce((sum, file) => sum + statSync(join(args.from, file)).size, 0);
console.log(`[package-llama] ${args.target}: ${kept.length}/${files.length} files, ${mb(bytes)} MB of ${mb(before)} MB`);
console.log(`[package-llama] kept: ${kept.join(" ")}`);
console.log(`[package-llama] dropped ${dropped.length}: ${dropped.slice(0, 6).join(" ")}${dropped.length > 6 ? " ..." : ""}`);
