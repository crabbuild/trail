import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import esbuild from "esbuild";

fs.rmSync("dist-test", { recursive: true, force: true });

await esbuild.build({
  entryPoints: [
    "src/test/acpCapabilities.test.ts",
    "src/test/acpClient.test.ts",
    "src/test/acpRenderReducers.test.ts",
    "src/test/conflicts.test.ts",
    "src/test/coordinationSummary.test.ts",
    "src/test/trailDaemonClient.test.ts",
    "src/test/trailHydration.test.ts",
    "src/test/mergeQueue.test.ts",
    "src/test/promptAttachment.test.ts",
    "src/test/promptCompletion.test.ts",
    "src/test/renderStreamScheduler.test.ts",
    "src/test/resourceTargets.test.ts",
    "src/test/securityRedaction.test.ts",
    "src/test/settingsModel.test.ts",
    "src/test/shellCommand.test.ts",
    "src/test/taskTreeModel.test.ts",
    "src/test/taskOverlaps.test.ts",
    "src/test/webviewApprovalModel.test.ts",
    "src/test/webviewApprovalCard.test.ts",
    "src/test/webviewBuild.test.ts",
    "src/test/webviewComposerCard.test.ts",
    "src/test/webviewComposerModel.test.ts",
    "src/test/webviewContentTextModel.test.ts",
    "src/test/webviewDiffCard.test.ts",
    "src/test/webviewDiffModel.test.ts",
    "src/test/webviewEventModel.test.ts",
    "src/test/webviewFilePreviewModel.test.ts",
    "src/test/webviewEmptyStateCard.test.ts",
    "src/test/webviewEventCard.test.ts",
    "src/test/webviewHeaderBar.test.ts",
    "src/test/webviewMarkdownModel.test.ts",
    "src/test/webviewMessageCard.test.ts",
    "src/test/webviewPayloadDisclosure.test.ts",
    "src/test/webviewPlanCard.test.ts",
    "src/test/webviewRecoveryBanner.test.ts",
    "src/test/webviewRenderPatchModel.test.ts",
    "src/test/webviewResultDrawer.test.ts",
    "src/test/webviewReviewDrawer.test.ts",
    "src/test/webviewReviewModel.test.ts",
    "src/test/webviewTerminalCard.test.ts",
    "src/test/webviewTimelineGroup.test.ts",
    "src/test/webviewTimelineNavigation.test.ts",
    "src/test/webviewTimelineScroller.test.ts",
    "src/test/webviewTerminalModel.test.ts",
    "src/test/webviewThoughtCard.test.ts",
    "src/test/webviewToolCallGroupCard.test.ts",
    "src/test/webviewTimelineModel.test.ts",
    "src/test/webviewToolCallCard.test.ts",
    "src/test/webviewToolModel.test.ts",
    "src/test/webviewToolbarModel.test.ts"
  ],
  outdir: "dist-test",
  bundle: true,
  platform: "node",
  format: "esm",
  outExtension: {
    ".js": ".mjs"
  },
  external: [
    "lucide-react",
    "react",
    "react-dom",
    "react-dom/client",
    "react-dom/server",
    "react-dom/server.browser",
    "use-sync-external-store/shim",
    "use-sync-external-store/shim/with-selector"
  ],
  sourcemap: true,
  target: "es2022",
  jsx: "automatic",
  alias: {
    "@": path.resolve("src")
  },
  logLevel: "info"
});

const testFiles = fs
  .readdirSync("dist-test")
  .filter((file) => file.endsWith(".mjs"))
  .map((file) => `dist-test/${file}`);

const result = spawnSync(process.execPath, ["--test", "--test-concurrency=1", ...testFiles], {
  stdio: "inherit"
});

process.exit(result.status ?? 1);
