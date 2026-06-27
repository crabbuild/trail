import esbuild from "esbuild";

const watch = process.argv.includes("--watch");

const common = {
  bundle: true,
  sourcemap: true,
  logLevel: "info",
  target: "es2022"
};

const builds = [
  {
    ...common,
    entryPoints: ["src/extension.ts"],
    outfile: "dist/extension.js",
    external: ["vscode"],
    platform: "node",
    format: "cjs"
  },
  {
    ...common,
    entryPoints: ["src/webview/main.ts"],
    outfile: "dist/webview.js",
    platform: "browser",
    format: "iife"
  }
];

if (watch) {
  const contexts = await Promise.all(builds.map((options) => esbuild.context(options)));
  await Promise.all(contexts.map((context) => context.watch()));
  console.log("Watching CrabDB VS Code extension sources...");
} else {
  await Promise.all(builds.map((options) => esbuild.build(options)));
}
