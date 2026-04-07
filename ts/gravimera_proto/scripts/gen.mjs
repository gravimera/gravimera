import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

function collectProtoFiles(rootDir) {
  const out = [];
  const stack = [rootDir];
  while (stack.length > 0) {
    const dir = stack.pop();
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.isFile() && entry.name.endsWith(".proto")) {
        out.push(full);
      }
    }
  }
  return out;
}

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const pkgDir = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(pkgDir, "..", "..");

const protoRoot = path.join(repoRoot, "proto");
const outDir = path.join(pkgDir, "src", "gen");

const pluginPath = path.join(
  pkgDir,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "protoc-gen-es.cmd" : "protoc-gen-es",
);

if (!fs.existsSync(protoRoot)) {
  console.error(`proto root not found: ${protoRoot}`);
  process.exit(2);
}

if (!fs.existsSync(pluginPath)) {
  console.error(`protoc-gen-es not found at: ${pluginPath}`);
  console.error("Run `npm install` in ts/gravimera_proto first.");
  process.exit(2);
}

fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir, { recursive: true });

const protos = collectProtoFiles(protoRoot).sort();
if (protos.length === 0) {
  console.error(`no .proto files found under: ${protoRoot}`);
  process.exit(2);
}

const args = [
  `--plugin=protoc-gen-es=${pluginPath}`,
  "-I",
  protoRoot,
  "--es_out",
  outDir,
  "--es_opt",
  "target=ts,import_extension=js",
  ...protos,
];

const result = spawnSync("protoc", args, { stdio: "inherit" });
if (result.error?.code === "ENOENT") {
  console.error("`protoc` not found in PATH.");
  console.error("Install the protobuf compiler, then re-run `npm run gen`.");
  process.exit(2);
}
process.exit(result.status ?? 1);
