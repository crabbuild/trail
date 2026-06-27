import { spawnSync } from "node:child_process";
import fs from "node:fs";
import esbuild from "esbuild";

fs.rmSync("dist-test", { recursive: true, force: true });

await esbuild.build({
  entryPoints: [
    "src/test/acpCapabilities.test.ts",
    "src/test/acpClient.test.ts",
    "src/test/acpRenderReducers.test.ts",
    "src/test/conflicts.test.ts",
    "src/test/coordinationSummary.test.ts",
    "src/test/crabDbDaemonClient.test.ts",
    "src/test/crabDbHydration.test.ts",
    "src/test/mergeQueue.test.ts",
    "src/test/promptAttachment.test.ts",
    "src/test/promptCompletion.test.ts",
    "src/test/resourceTargets.test.ts",
    "src/test/securityRedaction.test.ts",
    "src/test/shellCommand.test.ts",
    "src/test/taskOverlaps.test.ts"
  ],
  outdir: "dist-test",
  bundle: true,
  platform: "node",
  format: "esm",
  outExtension: {
    ".js": ".mjs"
  },
  sourcemap: true,
  target: "es2022",
  logLevel: "info"
});

const testFiles = fs
  .readdirSync("dist-test")
  .filter((file) => file.endsWith(".mjs"))
  .map((file) => `dist-test/${file}`);

const result = spawnSync(process.execPath, ["--test", ...testFiles], {
  stdio: "inherit"
});

process.exit(result.status ?? 1);
