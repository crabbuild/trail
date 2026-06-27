import * as vscode from "vscode";
import { ProviderRegistry, type AcpProviderProfile } from "../acp/ProviderRegistry";
import { getExtensionConfig } from "../config";
import { redactString } from "../shared/securityRedaction";

export class SettingsPanel {
  private static current: SettingsPanel | undefined;

  static open(extensionUri: vscode.Uri, workspaceRoot: string): void {
    if (SettingsPanel.current) {
      SettingsPanel.current.panel.reveal(vscode.ViewColumn.Active);
      SettingsPanel.current.refresh();
      return;
    }

    const panel = vscode.window.createWebviewPanel(
      "crabdb.settings",
      "CrabDB Settings",
      vscode.ViewColumn.Active,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "dist")]
      }
    );
    const view = new SettingsPanel(extensionUri, workspaceRoot, panel);
    SettingsPanel.current = view;
    panel.onDidDispose(() => {
      if (SettingsPanel.current === view) {
        SettingsPanel.current = undefined;
      }
    });
    panel.webview.onDidReceiveMessage((message: { type?: string; scope?: string }) => {
      void view.handleMessage(message);
    });
    view.refresh();
  }

  private constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly workspaceRoot: string,
    private readonly panel: vscode.WebviewPanel
  ) {}

  private refresh(): void {
    this.panel.webview.html = this.html();
  }

  private async handleMessage(message: { type?: string; scope?: string }): Promise<void> {
    if (message.type === "openSettings") {
      if (message.scope === "workspace") {
        await vscode.commands.executeCommand("workbench.action.openWorkspaceSettings", "crabdb");
        return;
      }
      if (message.scope === "user") {
        await vscode.commands.executeCommand("workbench.action.openGlobalSettings", "crabdb");
        return;
      }
      await vscode.commands.executeCommand("workbench.action.openSettings", "crabdb");
      return;
    }

    if (message.type === "customProviders") {
      await vscode.commands.executeCommand("workbench.action.openSettings", "crabdb.customProviders");
      return;
    }

    if (message.type === "doctor") {
      await vscode.commands.executeCommand("crabdb.doctor");
      return;
    }

    if (message.type === "startDaemon") {
      await vscode.commands.executeCommand("crabdb.startDaemon");
    }
  }

  private html(): string {
    const config = getExtensionConfig();
    const providers = new ProviderRegistry(this.workspaceRoot).profiles();
    const defaultProvider = providers.find((provider) => provider.id === config.defaultProvider) ?? providers[0];
    const style = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "dist", "webview.css"));
    const nonce = nonceValue();
    const csp = [
      "default-src 'none'",
      `style-src ${this.panel.webview.cspSource}`,
      `script-src 'nonce-${nonce}'`
    ].join("; ");

    return `<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="Content-Security-Policy" content="${escapeHtml(csp)}">
  <link rel="stylesheet" href="${style}">
  <title>CrabDB Settings</title>
</head>
<body>
  <main class="settings-shell">
    <header class="settings-header">
      <div>
        <h1>Settings</h1>
        <p>Configure the agent workflow, review gates, provider routing, and local CrabDB safety controls.</p>
      </div>
      <div class="settings-header-actions">
        <button data-action="openSettings" data-scope="workspace">Workspace settings</button>
        <button data-action="openSettings" data-scope="user">User settings</button>
      </div>
    </header>
    <div class="settings-layout">
      <nav class="settings-nav" aria-label="Settings sections">
        ${settingsNavItem("Providers", "provider")}
        ${settingsNavItem("Agent behavior", "terminal")}
        ${settingsNavItem("Context", "file")}
        ${settingsNavItem("Review", "review")}
        ${settingsNavItem("Checkpoints", "history")}
        ${settingsNavItem("Display", "display")}
        ${settingsNavItem("Diagnostics", "diagnostics")}
      </nav>
      <section class="settings-content">
        <section id="providers" class="settings-section">
          <div class="settings-section-heading">
            <h2>Providers</h2>
            <button data-action="customProviders">Edit custom providers</button>
          </div>
          <div class="settings-metrics">
            ${metricCard("Default provider", defaultProvider?.label || config.defaultProvider || "Not configured")}
            ${metricCard("Configured providers", String(providers.length))}
            ${metricCard("Custom providers", String(config.customProviders.length))}
            ${metricCard("CrabDB CLI", config.crabdbPath)}
          </div>
          <div class="provider-list">
            ${providers.map(providerCard).join("") || emptyPanel("No ACP providers are configured.")}
          </div>
        </section>
        <section id="agent-behavior" class="settings-section">
          <h2>Agent behavior</h2>
          <div class="settings-card-grid">
            ${settingsCard("Session routing", "Provider sessions resume when supported; otherwise CrabDB starts a follow-up from the latest checkpoint.")}
            ${settingsCard("Modes and slash commands", "Live ACP modes, config options, and commands appear in the chat composer when the provider advertises them.")}
            ${settingsCard("Daemon", config.autoStartDaemon ? "CrabDB daemon starts automatically when no endpoint is discovered." : "Daemon auto-start is disabled for this workspace.")}
          </div>
          <div class="settings-inline-actions">
            <button data-action="doctor">Run doctor</button>
            <button data-action="startDaemon">Start daemon</button>
          </div>
        </section>
        <section id="context" class="settings-section">
          <h2>Context and attachments</h2>
          <div class="settings-card-grid">
            ${settingsCard("Editor context", "Attach selection, active file, diagnostics, changed files, terminal output, and CrabDB history directly from the composer.")}
            ${settingsCard("Capability aware", "Image, audio, and embedded context controls are shown according to the active provider capabilities.")}
            ${settingsCard("Redaction", "Raw details, terminal snippets, and command payloads are redacted before rendering in the webview.")}
          </div>
        </section>
        <section id="review" class="settings-section">
          <h2>Review and safety</h2>
          <div class="settings-card-grid">
            ${settingsCard("Review drawer", "Open review from the chat toolbar when you need readiness, changed paths, conflicts, gates, or transcript jump links.")}
            ${settingsCard("Apply workflow", "Dry-run apply, queue merge, rewind, and preserve failed attempt stay explicit actions.")}
            ${settingsCard("Coordination", "Parallel task overlap, stale bases, dirty workdirs, conflicts, approvals, tests, and evals roll up into the review state.")}
          </div>
        </section>
        <section id="checkpoints" class="settings-section">
          <h2>Checkpoints</h2>
          <div class="settings-card-grid">
            ${settingsCard("Turn checkpoint", "Completed turns show checkpoint status so a reviewer can tell whether work is durable or still provisional.")}
            ${settingsCard("Follow-up start", "Failed or switched-provider turns start from the current CrabDB checkpoint instead of assuming private provider context transfers.")}
            ${settingsCard("Rewind", "Rewind and preserve failed attempt actions remain explicit and available from review surfaces.")}
          </div>
        </section>
        <section id="display" class="settings-section">
          <h2>Display</h2>
          <div class="settings-card-grid">
            ${settingsCard("Theme native", "The UI uses VS Code theme tokens for light, dark, high contrast, and custom workbench themes.")}
            ${settingsCard("Composer frame", "The prompt box uses a stable border fallback so it stays visible even when a theme omits input border colors.")}
            ${settingsCard("Progressive details", "Raw JSON, long terminal output, and compatibility data stay behind details controls.")}
          </div>
        </section>
        <section id="diagnostics" class="settings-section">
          <h2>Advanced configuration</h2>
          <dl class="settings-facts">
            <div><dt>Workspace</dt><dd>${escapeHtml(this.workspaceRoot)}</dd></div>
            <div><dt>Default provider id</dt><dd>${escapeHtml(config.defaultProvider)}</dd></div>
            <div><dt>Auto-start daemon</dt><dd>${config.autoStartDaemon ? "enabled" : "disabled"}</dd></div>
            <div><dt>Configuration keys</dt><dd><code>crabdb.path</code>, <code>crabdb.defaultProvider</code>, <code>crabdb.autoStartDaemon</code>, <code>crabdb.customProviders</code></dd></div>
          </dl>
        </section>
      </section>
    </div>
  </main>
  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    document.addEventListener("click", (event) => {
      const action = event.target.closest("[data-action]");
      if (!action) return;
      event.preventDefault();
      vscode.postMessage({ type: action.dataset.action, scope: action.dataset.scope });
    });
  </script>
</body>
</html>`;
  }
}

function providerCard(provider: AcpProviderProfile): string {
  const command = redactString([provider.command, ...provider.args].join(" "));
  return `
    <article class="provider-card">
      <header>
        <div>
          <h3>${escapeHtml(provider.label)}</h3>
          <p>${escapeHtml(provider.id)}</p>
        </div>
        <span class="status status-${provider.crabdbBacked ? "ready" : "dirty"}">${provider.crabdbBacked ? "Durable" : "Raw ACP"}</span>
      </header>
      <dl class="settings-facts compact">
        <div><dt>Task names</dt><dd>${provider.supportsTaskName ? "supported" : "not advertised"}</dd></div>
        <div><dt>Checkpoint start</dt><dd>${provider.supportsFromRef ? "supported" : "not advertised"}</dd></div>
      </dl>
      <code class="provider-command">${escapeHtml(command)}</code>
    </article>
  `;
}

type SettingsIcon = "diagnostics" | "display" | "file" | "history" | "provider" | "review" | "terminal";

function settingsNavItem(label: string, icon: SettingsIcon): string {
  return `<a href="#${escapeClass(label)}">${settingsIcon(icon)}<span>${escapeHtml(label)}</span></a>`;
}

function settingsIcon(icon: SettingsIcon): string {
  const open = `<svg class="settings-nav-icon" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">`;
  switch (icon) {
    case "diagnostics":
      return `${open}<path d="M10 3l8 14H2L10 3z"/><path d="M10 8v4"/><path d="M10 15h.01"/></svg>`;
    case "display":
      return `${open}<rect x="3" y="4" width="14" height="10" rx="1.5"/><path d="M8 17h4"/><path d="M10 14v3"/></svg>`;
    case "file":
      return `${open}<path d="M6 3h6l4 4v10H6z"/><path d="M12 3v5h4"/></svg>`;
    case "history":
      return `${open}<path d="M4 5v4h4"/><path d="M5 9a6 6 0 1 0 2-4"/><path d="M10 7v4l3 2"/></svg>`;
    case "provider":
      return `${open}<path d="M7 8V5a3 3 0 0 1 6 0v3"/><rect x="5" y="8" width="10" height="8" rx="2"/><path d="M10 11v2"/></svg>`;
    case "review":
      return `${open}<path d="M6 4h8"/><path d="M6 8h8"/><path d="M6 12h5"/><path d="M4 4h.01"/><path d="M4 8h.01"/><path d="M4 12h.01"/><path d="M13 15l2 2 3-5"/></svg>`;
    case "terminal":
      return `${open}<path d="M4 6l4 4-4 4"/><path d="M10 14h6"/></svg>`;
    default:
      return `${open}<circle cx="10" cy="10" r="6"/></svg>`;
  }
}

function metricCard(label: string, value: string): string {
  return `
    <article class="settings-metric">
      <span>${escapeHtml(label)}</span>
      <strong>${escapeHtml(value)}</strong>
    </article>
  `;
}

function settingsCard(title: string, body: string): string {
  return `
    <article class="settings-card">
      <h3>${escapeHtml(title)}</h3>
      <p>${escapeHtml(body)}</p>
    </article>
  `;
}

function emptyPanel(text: string): string {
  return `<p class="muted">${escapeHtml(text)}</p>`;
}

function nonceValue(): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let result = "";
  for (let index = 0; index < 32; index += 1) {
    result += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  return result;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });
}

function escapeClass(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9_-]/g, "-");
}
