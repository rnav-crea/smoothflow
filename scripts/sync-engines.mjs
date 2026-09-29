// Copy the offline engines next to the dev binary so dev matches an install.
// tauri-build also copies `bundle.resources` into the cargo target dir, so this
// is a belt-and-braces path for a target dir that was cleaned or a build script
// that did not rerun. Never deletes anything.
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
  console.warn(`[sync-engines] ${from} is missing — dev will not have offline engines.`);
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
