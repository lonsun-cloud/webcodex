#!/usr/bin/env node
import { watch, readFileSync, writeFileSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, dirname, basename, relative } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { build } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const expectedFiles = ["admin.html", "admin.js", "admin.css"];

function parseArguments(argv) {
  let outputDirectory = resolve(root, "dist");
  let checkOnly = false;
  let watchMode = false;
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--check") checkOnly = true;
    else if (value === "--watch") watchMode = true;
    else if (value === "--admin-only") continue;
    else if (value === "--out-dir") {
      const target = argv[++index];
      if (!target || target.startsWith("--")) throw new Error("--out-dir requires a path");
      outputDirectory = resolve(root, target);
    } else throw new Error("unknown option: " + value);
  }
  if (checkOnly && watchMode) throw new Error("--check and --watch cannot be used together");
  return { outputDirectory, checkOnly, watchMode };
}

async function buildInto(outputDirectory) {
  await build({
    configFile: false,
    root,
    publicDir: false,
    plugins: [react()],
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
    build: {
      outDir: outputDirectory,
      emptyOutDir: false,
      target: "es2022",
      minify: "esbuild",
      cssCodeSplit: false,
      lib: {
        entry: resolve(root, "src/admin-react/main.tsx"),
        formats: ["es"],
        fileName: () => "admin.js",
        cssFileName: "admin",
      },
      rollupOptions: { output: { codeSplitting: false } },
    },
  });
  const html = readFileSync(resolve(root, "src/admin.html"), "utf8").replace(/\r\n/g, "\n");
  writeFileSync(resolve(outputDirectory, "admin.html"), html.endsWith("\n") ? html : html + "\n");
  // Vendor minification can leave whitespace-only lines that fail the repository's
  // whitespace check; normalizing them keeps checked-in assets deterministic.
  const scriptPath = resolve(outputDirectory, "admin.js");
  writeFileSync(scriptPath, readFileSync(scriptPath, "utf8").replace(/^[\t ]+$/gm, ""));
  for (const name of expectedFiles) {
    if (!existsSync(resolve(outputDirectory, name))) throw new Error("Admin build did not produce " + name);
  }
}

function compareOutputs(actualDirectory, expectedDirectory) {
  const drift = expectedFiles.filter((name) => {
    const actual = resolve(actualDirectory, name);
    const expected = resolve(expectedDirectory, name);
    return !existsSync(actual) || !readFileSync(actual).equals(readFileSync(expected));
  });
  if (drift.length) throw new Error(drift.join(", ") + " out of date; run: npm --prefix frontend run build");
}

async function runBuild(outputDirectory, checkOnly) {
  const start = Date.now();
  if (checkOnly) {
    const temporary = mkdtempSync(resolve(tmpdir(), "webcodex-admin-"));
    try { await buildInto(temporary); compareOutputs(outputDirectory, temporary); }
    finally { rmSync(temporary, { recursive: true, force: true }); }
  } else await buildInto(outputDirectory);
  console.log("[admin] " + (checkOnly ? "checked " : "built ") + (relative(root, outputDirectory) || basename(outputDirectory)) + " (" + (Date.now() - start) + "ms)");
}

async function main() {
  const { outputDirectory, checkOnly, watchMode } = parseArguments(process.argv.slice(2));
  await runBuild(outputDirectory, checkOnly);
  if (!watchMode) return;
  let timer;
  let running = false;
  let queued = false;
  const rebuild = async () => {
    if (running) { queued = true; return; }
    running = true;
    try { await runBuild(outputDirectory, false); }
    catch (error) { console.error("[admin] build failed: " + String(error)); }
    finally { running = false; if (queued) { queued = false; void rebuild(); } }
  };
  const watcher = watch(resolve(root, "src"), { recursive: true }, () => {
    clearTimeout(timer);
    timer = setTimeout(() => void rebuild(), 100);
  });
  console.log("[admin] watching src");
  const close = () => { clearTimeout(timer); watcher.close(); };
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
}

main().catch((error) => { console.error("[admin] build failed: " + String(error)); process.exitCode = 1; });
