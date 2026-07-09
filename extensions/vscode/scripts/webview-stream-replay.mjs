import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const root = process.cwd();
const webviewDist = path.join(root, "dist", "webview");
const chromePath =
  process.env.CHROME_PATH ||
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

if (!fs.existsSync(path.join(webviewDist, "main.js"))) {
  throw new Error("dist/webview/main.js not found. Run npm run compile first.");
}
if (typeof WebSocket !== "function") {
  throw new Error("This script requires a Node runtime with the built-in WebSocket client.");
}
if (!fs.existsSync(chromePath)) {
  throw new Error(`Chrome not found at ${chromePath}. Set CHROME_PATH to a Chromium executable.`);
}

const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "trail-stream-replay-"));
const profileDir = path.join(tempRoot, "profile");
const htmlPath = path.join(tempRoot, "index.html");
const html = `<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <link rel="stylesheet" href="${pathToFileURL(path.join(webviewDist, "main.css")).href}">
  <title>Trail Stream Replay</title>
</head>
<body>
  <main id="app" aria-label="Trail agent chat"></main>
  <script>
    window.__vscodeMessages = [];
    window.__vscodeState = undefined;
    window.acquireVsCodeApi = () => ({
      postMessage(message) { window.__vscodeMessages.push(message); },
      getState() { return window.__vscodeState; },
      setState(state) { window.__vscodeState = state; }
    });
  </script>
  <script type="module" src="${pathToFileURL(path.join(webviewDist, "main.js")).href}"></script>
</body>
</html>`;
fs.writeFileSync(htmlPath, html);

let chrome;

async function main() {
  const endpoint = await launchChrome();
  const cdp = await CdpClient.connect(endpoint);
  const { targetId } = await cdp.send("Target.createTarget", {
    url: pathToFileURL(htmlPath).href
  });
  const { sessionId } = await cdp.send("Target.attachToTarget", {
    targetId,
    flatten: true
  });
  const pageErrors = [];
  cdp.onEvent((event) => {
    if (event.sessionId !== sessionId) {
      return;
    }
    if (event.method === "Runtime.exceptionThrown") {
      pageErrors.push(event.params?.exceptionDetails?.text || event.params?.exceptionDetails?.exception?.description || "Runtime exception");
    }
    if (event.method === "Log.entryAdded") {
      const entry = event.params?.entry;
      if (entry?.level === "error") {
        pageErrors.push(entry.text || "Log error");
      }
    }
  });
  await cdp.send("Runtime.enable", {}, sessionId);
  await cdp.send("Log.enable", {}, sessionId);
  await cdp.send("Page.enable", {}, sessionId);
  await cdp.send(
    "Emulation.setDeviceMetricsOverride",
    {
      width: 960,
      height: 900,
      deviceScaleFactor: 1,
      mobile: false
    },
    sessionId
  );
  await cdp.waitForEvent("Page.loadEventFired", sessionId, 10_000).catch(() => undefined);
  const result = await evaluate(cdp, sessionId, replaySource(), 60_000);
  result.errors = [...pageErrors, ...(result.errors || [])];
  console.log(JSON.stringify(result, null, 2));
  const failedChecks = [
    "bottomStayedPinned",
    "streamingTextMatches",
    "completedMessageRendered",
    "assistantExitedStreamingRenderer",
    "thoughtTextMatches",
    "terminalUpdated",
    "planUpdated",
    "toolUpdated",
    "completionRendered",
    "timelineOrderRetained",
    "assistantBeforeCompletion",
    "completionIsLastTimelineItem"
  ].filter((key) => result[key] !== true);
  if (result.errors.length || failedChecks.length) {
    throw new Error(`Stream replay failed checks: ${[...failedChecks, ...result.errors].join(", ")}`);
  }
  await cdp.close();
}

async function launchChrome() {
  return new Promise((resolve, reject) => {
    const args = [
      "--headless=new",
      "--disable-gpu",
      "--disable-background-timer-throttling",
      "--disable-renderer-backgrounding",
      "--disable-features=CalculateNativeWinOcclusion",
      "--disable-dev-shm-usage",
      "--no-first-run",
      "--no-default-browser-check",
      "--remote-debugging-port=0",
      `--user-data-dir=${profileDir}`,
      "--allow-file-access-from-files",
      "--window-size=960,900",
      "about:blank"
    ];
    chrome = spawn(chromePath, args, {
      stdio: ["ignore", "ignore", "pipe"]
    });
    let stderr = "";
    const timer = setTimeout(() => {
      reject(new Error(`Timed out waiting for Chrome DevTools endpoint.\n${stderr}`));
    }, 10_000);
    chrome.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    chrome.stderr.setEncoding("utf8");
    chrome.stderr.on("data", (chunk) => {
      stderr += chunk;
      const match = stderr.match(/DevTools listening on (ws:\/\/\S+)/);
      if (match?.[1]) {
        clearTimeout(timer);
        resolve(match[1]);
      }
    });
    chrome.once("exit", (code, signal) => {
      clearTimeout(timer);
      reject(new Error(`Chrome exited before DevTools became ready: ${code ?? signal}\n${stderr}`));
    });
  });
}

class CdpClient {
  static async connect(endpoint) {
    const socket = new WebSocket(endpoint);
    const client = new CdpClient(socket);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", () => reject(new Error("Unable to open DevTools WebSocket")), { once: true });
    });
    return client;
  }

  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Set();
    this.socket.addEventListener("message", (event) => {
      const payload = JSON.parse(String(event.data));
      if (payload.id) {
        const pending = this.pending.get(payload.id);
        if (!pending) {
          return;
        }
        this.pending.delete(payload.id);
        if (payload.error) {
          pending.reject(new Error(payload.error.message || JSON.stringify(payload.error)));
        } else {
          pending.resolve(payload.result || {});
        }
        return;
      }
      for (const listener of this.listeners) {
        listener(payload);
      }
    });
  }

  send(method, params = {}, sessionId) {
    const id = this.nextId++;
    const message = { id, method, params };
    if (sessionId) {
      message.sessionId = sessionId;
    }
    this.socket.send(JSON.stringify(message));
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }

  onEvent(listener) {
    this.listeners.add(listener);
  }

  waitForEvent(method, sessionId, timeoutMs) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.listeners.delete(listener);
        reject(new Error(`Timed out waiting for ${method}`));
      }, timeoutMs);
      const listener = (event) => {
        if (event.method !== method || event.sessionId !== sessionId) {
          return;
        }
        clearTimeout(timer);
        this.listeners.delete(listener);
        resolve(event.params || {});
      };
      this.listeners.add(listener);
    });
  }

  close() {
    this.socket.close();
  }
}

async function evaluate(cdp, sessionId, expression, timeoutMs) {
  const result = await cdp.send(
    "Runtime.evaluate",
    {
      expression,
      awaitPromise: true,
      returnByValue: true,
      timeout: timeoutMs
    },
    sessionId
  );
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text || result.exceptionDetails.exception?.description || "Runtime.evaluate failed");
  }
  return result.result?.value;
}

function replaySource() {
  return `(${async function runReplay() {
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const raf = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));
    const domId = (id) => "node-" + id.toLowerCase().replace(/[^a-z0-9_-]/g, "-");
    const dispatch = (message) => window.dispatchEvent(new MessageEvent("message", { data: message }));
    const errors = [];
    window.addEventListener("error", (event) => errors.push(event.message || "window error"));
    window.addEventListener("unhandledrejection", (event) => errors.push(String(event.reason || "unhandled rejection")));

    let layoutShift = 0;
    let longTasks = 0;

    function baseNode(id, kind, extra = {}) {
      return {
        id,
        kind,
        taskId: "task-stream-replay",
        lane: "main",
        turnId: "turn-1",
        acpSessionId: "session-replay",
        provider: "Replay",
        source: "acp-live",
        status: "in_progress",
        createdAt: "2026-06-29T00:00:00.000Z",
        updatedAt: "2026-06-29T00:00:00.000Z",
        ...extra
      };
    }

    const ids = {
      assistant: "message:assistant:replay",
      thought: "thought:replay",
      plan: "plan:replay",
      tool: "tool:read:replay",
      terminal: "terminal:replay"
    };
    const tracked = [
      [ids.assistant, "[data-message-card-root]"],
      [ids.thought, "[data-thought-card-root]"],
      [ids.plan, "[data-plan-card-root]"],
      [ids.tool, "[data-tool-call-card-root]"],
      [ids.terminal, "[data-terminal-card-root]"]
    ];
    let revision = 1;
    let assistantText = "Starting stream.";
    let thoughtText = "Inspecting current rendering path.";
    let terminalText = "npm test -- --stream\\n";
    let toolText = "Reading current webview streaming code.";
    let planEntries = [
      { title: "Locate streaming hot path", status: "completed" },
      { title: "Patch direct island hydration", status: "in_progress" },
      { title: "Replay multi-component stream", status: "pending" }
    ];

    function nodes(frame = 0) {
      return [
        baseNode("message:user:replay", "message", {
          source: "user",
          status: "completed",
          role: "user",
          streaming: false,
          content: [{ type: "text", text: "Please optimize the streaming UX." }],
          text: "Please optimize the streaming UX."
        }),
        baseNode(ids.assistant, "message", {
          role: "assistant",
          streaming: true,
          content: [{ type: "text", text: assistantText }],
          text: assistantText
        }),
        baseNode(ids.thought, "thought", {
          ephemeral: true,
          content: [{ type: "text", text: thoughtText }]
        }),
        baseNode(ids.plan, "plan", {
          entries: planEntries
        }),
        baseNode(ids.tool, "tool", {
          toolCallId: "read-replay",
          title: "Read rendering files",
          toolKind: "read",
          toolStatus: "in_progress",
          locations: [{ path: "extensions/vscode/src/webview/main.ts", line: 1228 }],
          rawInput: { path: "extensions/vscode/src/webview/main.ts" },
          content: [{ type: "content", content: { type: "text", text: toolText } }]
        }),
        baseNode(ids.terminal, "terminal", {
          terminalId: "replay",
          title: "Run stream replay",
          command: "npm test",
          status: "in_progress",
          stdout: terminalText,
          elapsedMs: frame * 16
        })
      ];
    }

    dispatch({
      type: "state",
      renderRevision: revision,
      task: {
        id: "task-stream-replay",
        lane: "main",
        title: "Streaming replay",
        status: "running",
        provider: "Replay"
      },
      nodes: nodes(),
      attachments: [],
      sending: true,
      provider: "Replay",
      providerId: "replay",
      providers: [],
      taskOverlaps: [],
      capabilities: {},
      permissionPending: false
    });

    const waitFor = async (predicate, label, timeout = 6000) => {
      const start = performance.now();
      while (performance.now() - start < timeout) {
        if (predicate()) {
          return;
        }
        await sleep(16);
      }
      throw new Error("Timed out waiting for " + label);
    };
    await waitFor(
      () => tracked.every(([id, selector]) => document.getElementById(domId(id))?.querySelector(selector)),
      "initial island hydration"
    );
    await raf();
    await raf();

    const articleRefs = new Map();
    const rootRefs = new Map();
    for (const [id, selector] of tracked) {
      const article = document.getElementById(domId(id));
      articleRefs.set(id, article);
      rootRefs.set(id, article?.querySelector(selector));
    }

    const timeline = document.querySelector(".timeline");
    if (timeline) {
      timeline.scrollTop = timeline.scrollHeight;
    }
    try {
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (!entry.hadRecentInput) {
            layoutShift += entry.value || 0;
          }
        }
      }).observe({ type: "layout-shift" });
    } catch {}
    try {
      new PerformanceObserver((list) => {
        longTasks += list.getEntries().length;
      }).observe({ type: "longtask" });
    } catch {}
    let removedTrackedNodes = 0;
    let addedTrackedNodes = 0;
    const trackedDomIds = new Set(tracked.map(([id]) => domId(id)));
    const isTrackedElement = (node) => {
      if (!(node instanceof Element)) {
        return false;
      }
      if (trackedDomIds.has(node.id)) {
        return true;
      }
      return [...trackedDomIds].some((id) => node.querySelector?.("#" + CSS.escape(id)));
    };
    const observer = new MutationObserver((mutations) => {
      for (const mutation of mutations) {
        for (const node of mutation.removedNodes) {
          if (isTrackedElement(node)) {
            removedTrackedNodes += 1;
          }
        }
        for (const node of mutation.addedNodes) {
          if (isTrackedElement(node)) {
            addedTrackedNodes += 1;
          }
        }
      }
    });
    observer.observe(document.querySelector(".timeline-scroller-content") || document.body, {
      childList: true,
      subtree: true
    });

    const samples = [];
    let sampling = true;
    let blankFrames = 0;
    let articleRemounts = 0;
    let rootRemounts = 0;
    let maxBottomDrift = 0;
    const sample = () => {
      if (!sampling) {
        return;
      }
      let blank = false;
      for (const [id, selector] of tracked) {
        const article = document.getElementById(domId(id));
        const root = article?.querySelector(selector);
        if (!article || !root || !article.isConnected || !root.isConnected) {
          blank = true;
        }
        if (article && articleRefs.get(id) !== article) {
          articleRemounts += 1;
          articleRefs.set(id, article);
        }
        if (root && rootRefs.get(id) !== root) {
          rootRemounts += 1;
          rootRefs.set(id, root);
        }
      }
      if (blank) {
        blankFrames += 1;
      }
      const scroller = document.querySelector(".timeline");
      const drift = scroller ? Math.abs(scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight) : 0;
      maxBottomDrift = Math.max(maxBottomDrift, drift);
      samples.push({ blank, drift });
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);

    const frames = 90;
    for (let frame = 1; frame <= frames; frame += 1) {
      assistantText += "\\n" + String(frame).padStart(2, "0") + " streamed token text with stable DOM.";
      thoughtText += "\\nreasoning tick " + frame;
      terminalText += "frame " + frame + ": replay output stayed mounted\\n";
      toolText = "Streaming read progress: " + frame + " chunks inspected.";
      planEntries = planEntries.map((entry, index) =>
        index === 1
          ? { ...entry, title: "Patch direct island hydration (" + frame + "/90)", status: "in_progress" }
          : entry
      );
      const nextNodes = nodes(frame);
      dispatch({
        type: "renderPatches",
        baseRenderRevision: revision,
        renderRevision: revision + 1,
        sending: true,
        permissionPending: false,
        patches: [
          { type: "upsert", node: nextNodes.find((node) => node.id === ids.assistant) },
          { type: "upsert", node: nextNodes.find((node) => node.id === ids.thought) },
          { type: "upsert", node: nextNodes.find((node) => node.id === ids.plan) },
          { type: "upsert", node: nextNodes.find((node) => node.id === ids.tool) },
          { type: "upsert", node: nextNodes.find((node) => node.id === ids.terminal) }
        ]
      });
      revision += 1;
      await sleep(8);
    }
    const finalNodes = nodes(frames).map((node) => {
      if (node.id === ids.assistant && node.kind === "message") {
        return { ...node, status: "completed", streaming: false };
      }
      if (node.id === ids.thought || node.id === ids.plan || node.id === ids.tool || node.id === ids.terminal) {
        return { ...node, status: "completed", toolStatus: node.id === ids.tool ? "completed" : node.toolStatus, terminalStatus: node.id === ids.terminal ? "completed" : node.terminalStatus };
      }
      return node;
    });
    dispatch({
      type: "renderPatches",
      baseRenderRevision: revision,
      renderRevision: revision + 1,
      sending: false,
      permissionPending: false,
      patches: [
        { type: "upsert", node: finalNodes.find((node) => node.id === ids.assistant) },
        { type: "upsert", node: finalNodes.find((node) => node.id === ids.thought) },
        { type: "upsert", node: finalNodes.find((node) => node.id === ids.plan) },
        { type: "upsert", node: finalNodes.find((node) => node.id === ids.tool) },
        { type: "upsert", node: finalNodes.find((node) => node.id === ids.terminal) },
        {
          type: "upsert",
          node: baseNode("completion:replay", "completion", {
            status: "completed",
            stopReason: "end_turn",
            label: "Turn complete; checkpoint pending",
            checkpointPending: true
          })
        }
      ]
    });
    revision += 1;
    await sleep(350);
    sampling = false;
    observer.disconnect();
    await raf();

    const streamingMarkdownText = (id) => {
      const target = document.getElementById(domId(id))?.querySelector("[data-streaming-markdown]");
      return target?.__trailStreamingText || target?.textContent || "";
    };
    const finalText = streamingMarkdownText(ids.assistant);
    const finalThought = streamingMarkdownText(ids.thought);
    const finalMessageText = document.getElementById(domId(ids.assistant))?.textContent || "";
    const finalThoughtText = document.getElementById(domId(ids.thought))?.textContent || "";
    const finalTerminal = document.getElementById(domId(ids.terminal))?.textContent || "";
    const finalPlan = document.getElementById(domId(ids.plan))?.textContent || "";
    const finalTool = document.getElementById(domId(ids.tool))?.textContent || "";
    const finalCompletion = document.getElementById(domId("completion:replay"))?.textContent || "";
    const finalScroller = document.querySelector(".timeline");
    const finalBottomDrift = finalScroller ? Math.abs(finalScroller.scrollHeight - finalScroller.scrollTop - finalScroller.clientHeight) : 0;
    const finalTimelineOrder = [...document.querySelectorAll("[data-timeline-group-body-item]")]
      .map((element) => element.getAttribute("data-node-id") || "")
      .filter(Boolean);
    const expectedTimelineOrder = [
      "message:user:replay",
      ids.assistant,
      ids.thought,
      ids.plan,
      ids.tool,
      ids.terminal,
      "completion:replay"
    ];
    const timelineOrderRetained =
      finalTimelineOrder.length === expectedTimelineOrder.length &&
      expectedTimelineOrder.every((id, index) => finalTimelineOrder[index] === id);
    const assistantIndex = finalTimelineOrder.indexOf(ids.assistant);
    const completionIndex = finalTimelineOrder.indexOf("completion:replay");

    return {
      frames,
      samples: samples.length,
      finalTimelineOrder,
      addedTrackedNodes,
      removedTrackedNodes,
      articleRemounts,
      rootRemounts,
      blankFrames,
      maxBottomDrift,
      finalBottomDrift,
      bottomStayedPinned: finalBottomDrift < 48,
      longTasks,
      layoutShift,
      streamingTextMatches: finalText === assistantText || finalMessageText.includes("90 streamed token text with stable DOM."),
      completedMessageRendered: finalMessageText.includes("Starting stream.") && finalMessageText.includes("90 streamed token text with stable DOM."),
      assistantExitedStreamingRenderer: !document.getElementById(domId(ids.assistant))?.querySelector("[data-streaming-markdown]"),
      thoughtTextMatches: finalThought === thoughtText || finalThoughtText.includes("reasoning tick 90"),
      terminalUpdated: finalTerminal.includes("frame 90"),
      planUpdated: finalPlan.includes("90/90"),
      toolUpdated: finalTool.includes("Read rendering files"),
      completionRendered: finalCompletion.includes("Turn complete") || finalCompletion.includes("Completed"),
      timelineOrderRetained,
      assistantBeforeCompletion: assistantIndex >= 0 && completionIndex >= 0 && assistantIndex < completionIndex,
      completionIsLastTimelineItem: finalTimelineOrder.at(-1) === "completion:replay",
      errors
    };
  }}())`;
}

try {
  await main();
} finally {
  if (chrome && !chrome.killed) {
    chrome.kill("SIGTERM");
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, 1000);
      chrome.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
  }
  fs.rmSync(tempRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
