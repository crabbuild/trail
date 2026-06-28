import esbuild from "esbuild";
import { watch as watchFile } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";

const watch = process.argv.includes("--watch");
const isWindows = process.platform === "win32";
const tailwindBin = path.join("node_modules", ".bin", isWindows ? "tailwindcss.cmd" : "tailwindcss");
const tailwindInput = path.join("src", "webview", "tailwind.css");
const legacyCssInput = path.join("src", "webview", "styles.css");
const tailwindOutput = path.join("dist", "webview", "tailwind.css");
const webviewCssOutput = path.join("dist", "webview", "main.css");

const common = {
  bundle: true,
  sourcemap: true,
  logLevel: "info",
  target: "es2022",
  jsx: "automatic",
  alias: {
    "@": path.resolve("src")
  }
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
    outdir: "dist/webview",
    platform: "browser",
    format: "esm",
    minifySyntax: true,
    splitting: true,
    entryNames: "[name]",
    chunkNames: "chunks/[name]-[hash]"
  }
];

async function buildCss({ watch = false } = {}) {
  await mkdir(path.join("dist", "webview"), { recursive: true });
  if (watch) {
    await writeFile(tailwindOutput, "", { flag: "a" });
  }
  const args = [
    "-i",
    tailwindInput,
    "-o",
    tailwindOutput
  ];
  if (watch) {
    args.push("--watch=always");
  }
  const child = spawn(tailwindBin, args, { stdio: "inherit" });
  if (watch) {
    const combine = () => {
      void combineCss().catch((error) => {
        console.error(error);
      });
    };
    const tailwindWatcher = watchFile(tailwindOutput, combine);
    const legacyWatcher = watchFile(legacyCssInput, combine);
    setTimeout(combine, 500);
    child.on("exit", (code) => {
      tailwindWatcher.close();
      legacyWatcher.close();
      if (code && code !== 0) {
        process.exitCode = code;
      }
    });
    return child;
  }
  await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`Tailwind CSS build failed with exit code ${code}`));
      }
    });
  });
  await combineCss();
}

async function combineCss() {
  const [tailwindCss, legacyCss] = await Promise.all([
    readFile(tailwindOutput, "utf8").catch(() => ""),
    readFile(legacyCssInput, "utf8")
  ]);
  await writeFile(webviewCssOutput, `${tailwindCss}\n${legacyCss}`);
}

if (watch) {
  const contexts = await Promise.all(builds.map((options) => esbuild.context(options)));
  await Promise.all(contexts.map((context) => context.watch()));
  await buildCss({ watch: true });
  console.log("Watching CrabDB VS Code extension sources...");
} else {
  await Promise.all([...builds.map((options) => esbuild.build(options)), buildCss()]);
}
