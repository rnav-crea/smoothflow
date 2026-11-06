// Mirror a locally built engine tree next to the dev binary.
//
// The engines no longer ship in the installer: the app downloads them on
// demand into `<config dir>/engines/<target>/`, which `stt_local` and
// `llm_local` prefer. This script only covers the fallback path — the
// executable's own directory — so a dev machine that has a `binaries/win-x64`
// tree (left over from a manual build of `native/smoothflow-stt` plus a
// packaged llama runtime) gets it copied into `target/debug` without anyone
// touching the config directory by hand. A dev machine without one downloads
// the engines through the app like any user.
//
// Never deletes anything, and never fails the dev server: a missing source is
// only a warning.
import { cpSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const from = join(root, "src-tauri", "binaries", "win-x64");
const to = join(root, "src-tauri", "target", "debug");

if (process.platform !== "win32") {
  console.log("[sync-engines] not Windows — offline engines are Windows-only for now, skipping.");
  process.exit(0);
}

if (!existsSync(from)) {
  console.log(
    `[sync-engines] ${from} is not present — the app will download the engines into its config directory instead.`,
  );
  process.exit(0);
}

for (const entry of ["smoothflow-stt.exe", "llama"]) {
  const source = join(from, entry);
  if (!existsSync(source)) {
    console.warn(`[sync-engines] ${source} is missing — skipping.`);
    continue;
  }
  cpSync(source, join(to, entry), { recursive: true, force: true });
  console.log(`[sync-engines] ${entry} -> ${join(to, entry)}`);
}
