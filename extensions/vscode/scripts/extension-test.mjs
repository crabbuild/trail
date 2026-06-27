import path from "node:path";
import { fileURLToPath } from "node:url";
import fs from "node:fs";
import os from "node:os";
import { spawnSync } from "node:child_process";
import esbuild from "esbuild";
import { runTests } from "@vscode/test-electron";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testOut = path.join(root, "dist-test", "vscode");
const workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "crabdb-vscode-test-"));

fs.writeFileSync(path.join(workspaceRoot, "README.md"), "hello from vscode extension test\n");
const init = spawnSync("crabdb", ["--workspace", workspaceRoot, "init", "--quiet"], {
  encoding: "utf8"
});
if (init.status !== 0) {
  fs.rmSync(workspaceRoot, { recursive: true, force: true });
  throw new Error(`Failed to initialize CrabDB test workspace:\n${init.stdout}\n${init.stderr}`);
}

await esbuild.build({
  entryPoints: ["src/test/vscode/suite/index.ts"],
  outfile: path.join(testOut, "index.js"),
  bundle: true,
  platform: "node",
  format: "cjs",
  external: ["vscode"],
  sourcemap: true,
  target: "es2022",
  logLevel: "info"
});

try {
  await runTests({
    extensionDevelopmentPath: root,
    extensionTestsPath: path.join(testOut, "index.js"),
    launchArgs: [workspaceRoot, "--disable-extensions"],
    extensionTestsEnv: {
      CRABDB_VSCODE_TEST_WORKSPACE: workspaceRoot
    }
  });
} finally {
  fs.rmSync(workspaceRoot, { recursive: true, force: true });
}
