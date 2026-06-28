import * as vscode from "vscode";
import { ProviderRegistry } from "../acp/ProviderRegistry";
import { getExtensionConfig } from "../config";
import { redactString } from "../shared/securityRedaction";
import {
  buildSettingsViewModel,
  type ProviderCapabilityView,
  type ProviderRoutingFact,
  type ProviderRoutingSummary,
  type SettingsAction,
  type SettingsMetric,
  type SettingsNextStep,
  type SettingsRow,
  type SettingsSectionIcon,
  type SettingsSectionView
} from "./settingsModel";

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
    panel.webview.onDidReceiveMessage((message: { type?: string; scope?: string; key?: string }) => {
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

  private async handleMessage(message: { type?: string; scope?: string; key?: string }): Promise<void> {
    if (message.type === "openSettings") {
      if (message.key) {
        await vscode.commands.executeCommand("workbench.action.openSettings", message.key);
        return;
      }
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
    const model = buildSettingsViewModel(config, providers);
    const sectionMap = new Map(model.sections.map((section) => [section.id, section]));
    const style = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "dist", "webview", "main.css"));
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
      <div class="settings-header-title">
        <h1>Settings</h1>
        <p>Configure provider routing, local safety controls, context capture, review gates, and display behavior.</p>
      </div>
      <div class="settings-header-tools">
        <label class="settings-search">
          <span class="sr-only">Filter settings</span>
          <input data-settings-filter type="search" aria-controls="settings-content" autocomplete="off" spellcheck="false" placeholder="Filter settings, providers, capabilities...">
          <button type="button" data-settings-filter-clear hidden title="Clear settings filter">Clear</button>
        </label>
        <div class="settings-header-actions">
          ${model.secondaryActions.slice(2).map((action) => settingsActionButton(action)).join("")}
        </div>
      </div>
    </header>
    <div class="settings-layout">
      <nav class="settings-nav" aria-label="Settings sections">
        ${model.sections.map((section, index) => settingsNavItem(section, index === 0)).join("")}
      </nav>
      <section id="settings-content" class="settings-content">
        <p class="settings-filter-status" data-settings-filter-status role="status">Showing all settings sections.</p>
        <div class="settings-filter-empty" data-settings-filter-empty hidden>
          <p>No settings sections match this filter.</p>
          <button type="button" data-settings-filter-clear>Clear filter</button>
        </div>
        <section ${settingsSectionAttrs(sectionMap.get("overview"))} class="settings-section">
          <div class="settings-overview">
            <article class="settings-hero-card settings-health-${escapeClass(model.healthTone)}">
              <span class="settings-kicker">CrabDB control plane</span>
              <h2>${escapeHtml(model.healthLabel)}</h2>
              <p>${escapeHtml(model.healthDetail)}</p>
              <div class="settings-hero-actions">
                ${settingsActionButton(model.primaryAction, true)}
                ${model.secondaryActions.slice(0, 2).map((action) => settingsActionButton(action)).join("")}
              </div>
            </article>
            <div class="settings-metrics settings-overview-metrics">
              ${model.metrics.map(metricCard).join("")}
            </div>
          </div>
          ${settingsNextSteps(model.nextSteps)}
          ${settingsIssuePanel(model.issues)}
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("providers"))} class="settings-section">
          <div class="settings-section-heading">
            <h2>Providers</h2>
            <div class="settings-inline-actions">
              ${settingsActionButton({ type: "openSettings", key: "crabdb.defaultProvider", label: "Change default", detail: "Choose the default ACP provider." })}
              ${settingsActionButton({ type: "customProviders", label: "Edit custom providers", detail: "Open custom provider configuration." })}
            </div>
          </div>
          ${providerRoutingPanel(model.providerRouting)}
          <div class="settings-metrics">
            ${metricCard({ label: "Configured providers", value: String(model.providerCoverage.total), detail: "Built-in and custom ACP provider routes.", tone: model.providerCoverage.total ? "ok" : "warn" })}
            ${metricCard({ label: "Durable routes", value: String(model.providerCoverage.durable), detail: "Providers routed through CrabDB.", tone: model.providerCoverage.durable ? "ok" : "warn" })}
            ${metricCard({ label: "Custom providers", value: String(model.providerCoverage.custom), detail: "Workspace or user supplied provider commands.", tone: model.providerCoverage.custom ? "ok" : "neutral" })}
            ${metricCard({ label: "CrabDB CLI", value: config.crabdbPath || "missing", detail: "Executable used by relay and daemon commands.", tone: config.crabdbPath ? "ok" : "warn" })}
          </div>
          <div class="provider-list">
            ${model.providers.map(providerCard).join("") || emptyPanel("No ACP providers are configured. Add a provider before starting an agent task.")}
          </div>
          <div class="provider-matrix" role="table" aria-label="Provider capability matrix">
            <div class="provider-matrix-row provider-matrix-head" role="row">
              <span role="columnheader">Provider</span>
              <span role="columnheader">CrabDB durable</span>
              <span role="columnheader">Task names</span>
              <span role="columnheader">Checkpoint start</span>
            </div>
            ${model.providers.map(providerMatrixRow).join("")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("configuration"))} class="settings-section">
          <div class="settings-section-heading">
            <h2>Configuration</h2>
            ${settingsActionButton({ type: "openSettings", label: "Open all CrabDB settings", detail: "Open the CrabDB settings query." })}
          </div>
          <div class="settings-config-list">
            ${model.rows.map(settingRow).join("")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("agent-behavior"))} class="settings-section">
          <h2>Agent behavior</h2>
          <div class="settings-card-grid">
            ${settingsCard("Session routing", "Provider sessions resume when supported; otherwise CrabDB starts a follow-up from the latest checkpoint.")}
            ${settingsCard("Modes and slash commands", "Live ACP modes, config options, and commands appear in the chat composer when the provider advertises them.")}
            ${settingsCard("Daemon", config.autoStartDaemon ? "CrabDB daemon starts automatically when no endpoint is discovered." : "Daemon auto-start is disabled for this workspace.")}
          </div>
          <div class="settings-inline-actions">
            ${settingsActionButton({ type: "doctor", label: "Run doctor", detail: "Check the local CrabDB toolchain." })}
            ${settingsActionButton({ type: "startDaemon", label: "Start daemon", detail: "Bring up CrabDB services for review and queue state." })}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("context"))} class="settings-section">
          <h2>Context and attachments</h2>
          <div class="settings-card-grid">
            ${settingsCard("Editor context", "Attach selection, active file, diagnostics, changed files, terminal output, and CrabDB history directly from the composer.")}
            ${settingsCard("Capability aware", "Image, audio, and embedded context controls are shown according to the active provider capabilities.")}
            ${settingsCard("Redaction", "Raw details, terminal snippets, and command payloads are redacted before rendering in the webview.")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("review"))} class="settings-section">
          <h2>Review and safety</h2>
          <div class="settings-card-grid">
            ${settingsCard("Review drawer", "Open review from the chat toolbar when you need readiness, changed paths, conflicts, gates, or transcript jump links.")}
            ${settingsCard("Apply workflow", "Dry-run apply, queue merge, rewind, and preserve failed attempt stay explicit actions.")}
            ${settingsCard("Coordination", "Parallel task overlap, stale bases, dirty workdirs, conflicts, approvals, tests, and evals roll up into the review state.")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("checkpoints"))} class="settings-section">
          <h2>Checkpoints</h2>
          <div class="settings-card-grid">
            ${settingsCard("Turn checkpoint", "Completed turns show checkpoint status so a reviewer can tell whether work is durable or still provisional.")}
            ${settingsCard("Follow-up start", "Failed or switched-provider turns start from the current CrabDB checkpoint instead of assuming private provider context transfers.")}
            ${settingsCard("Rewind", "Rewind and preserve failed attempt actions remain explicit and available from review surfaces.")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("display"))} class="settings-section">
          <h2>Display</h2>
          <div class="settings-card-grid">
            ${settingsCard("Theme native", "The UI uses VS Code theme tokens for light, dark, high contrast, and custom workbench themes.")}
            ${settingsCard("Composer frame", "The prompt box uses a stable border fallback so it stays visible even when a theme omits input border colors.")}
            ${settingsCard("Diff and code previews", "File changes render as structured diffs, while file and resource previews use Shiki-backed tokenization with safe fallbacks.")}
            ${settingsCard("Progressive details", "Raw JSON, long terminal output, and compatibility data stay behind details controls.")}
          </div>
        </section>
        <section ${settingsSectionAttrs(sectionMap.get("diagnostics"))} class="settings-section">
          <h2>Advanced configuration</h2>
          <dl class="settings-facts">
            <div><dt>Workspace</dt><dd>${escapeHtml(this.workspaceRoot)}</dd></div>
            <div><dt>Default provider id</dt><dd>${escapeHtml(config.defaultProvider)}</dd></div>
            <div><dt>Auto-start daemon</dt><dd>${config.autoStartDaemon ? "enabled" : "disabled"}</dd></div>
            <div><dt>Provider durability</dt><dd>${model.providerCoverage.durable} durable of ${model.providerCoverage.total} configured</dd></div>
            <div><dt>Capability coverage</dt><dd>${model.providerCoverage.taskNames} task-name routes, ${model.providerCoverage.checkpoints} checkpoint-start routes</dd></div>
            <div><dt>Configuration keys</dt><dd><code>crabdb.path</code>, <code>crabdb.defaultProvider</code>, <code>crabdb.autoStartDaemon</code>, <code>crabdb.customProviders</code></dd></div>
          </dl>
        </section>
      </section>
    </div>
  </main>
  <script nonce="${nonce}">
    const vscode = acquireVsCodeApi();
    const settingsFilter = document.querySelector("[data-settings-filter]");
    const settingsFilterStatus = document.querySelector("[data-settings-filter-status]");
    const settingsFilterEmpty = document.querySelector("[data-settings-filter-empty]");
    const settingsFilterClearButtons = Array.from(document.querySelectorAll("[data-settings-filter-clear]"));
    const settingsSections = Array.from(document.querySelectorAll("[data-settings-section]"));
    const settingsNav = document.querySelector(".settings-nav");
    const settingsNavItems = Array.from(document.querySelectorAll("[data-settings-nav-target]"));

    function normalizeSettingsFilter(value) {
      return String(value || "").toLowerCase().replace(/\\s+/g, " ").trim();
    }

    function settingsFilterTokens(value) {
      return normalizeSettingsFilter(value).split(" ").filter(Boolean);
    }

    function settingsSectionMatches(section, tokens) {
      const searchable = normalizeSettingsFilter(section.dataset.settingsSearch || section.textContent);
      return tokens.every((token) => searchable.includes(token));
    }

    function visibleSettingsNavItems() {
      return settingsNavItems.filter((item) => item.getAttribute("aria-disabled") !== "true");
    }

    function settingsNavItemForSection(section) {
      return settingsNavItems.find((item) => item.dataset.settingsNavTarget === section?.dataset.settingsSectionId);
    }

    function setActiveSettingsNavItem(activeItem) {
      settingsNavItems.forEach((item) => {
        const isActive = item === activeItem;
        item.classList.toggle("settings-nav-active", isActive);
        if (isActive) {
          item.setAttribute("aria-current", "page");
        } else {
          item.removeAttribute("aria-current");
        }
      });
    }

    function updateSettingsNavFromScroll() {
      const visibleSections = settingsSections.filter((section) => !section.hidden);
      if (!visibleSections.length) return;
      const anchorY = Math.min(180, Math.max(72, window.innerHeight * 0.28));
      let activeSection = visibleSections[0];
      visibleSections.forEach((section) => {
        if (section.getBoundingClientRect().top <= anchorY) {
          activeSection = section;
        }
      });
      const activeItem = settingsNavItemForSection(activeSection);
      if (activeItem) {
        setActiveSettingsNavItem(activeItem);
      }
    }

    let settingsNavScrollPending = false;

    function scheduleSettingsNavFromScroll() {
      if (settingsNavScrollPending) return;
      settingsNavScrollPending = true;
      requestAnimationFrame(() => {
        settingsNavScrollPending = false;
        updateSettingsNavFromScroll();
      });
    }

    function currentSettingsNavItem() {
      return settingsNavItems.find((item) => item.getAttribute("aria-current") === "page" && item.getAttribute("aria-disabled") !== "true") || visibleSettingsNavItems()[0];
    }

    function focusSettingsNavItem(item, scrollSection) {
      if (!item) return;
      setActiveSettingsNavItem(item);
      item.focus({ preventScroll: true });
      if (!scrollSection) return;
      const section = document.getElementById(item.dataset.settingsNavTarget || "");
      section?.scrollIntoView({ block: "start", inline: "nearest" });
      if (item.hash) {
        history.replaceState(null, "", item.hash);
      }
    }

    function moveSettingsNavFocus(delta) {
      const visibleItems = visibleSettingsNavItems();
      if (!visibleItems.length) return;
      const current = currentSettingsNavItem();
      const currentIndex = Math.max(0, visibleItems.indexOf(current));
      const nextIndex = (currentIndex + delta + visibleItems.length) % visibleItems.length;
      focusSettingsNavItem(visibleItems[nextIndex], true);
    }

    function handleSettingsNavKeydown(event) {
      if (event.key === "ArrowDown" || event.key === "ArrowRight") {
        event.preventDefault();
        moveSettingsNavFocus(1);
      } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
        event.preventDefault();
        moveSettingsNavFocus(-1);
      } else if (event.key === "Home") {
        event.preventDefault();
        focusSettingsNavItem(visibleSettingsNavItems()[0], true);
      } else if (event.key === "End") {
        event.preventDefault();
        const visibleItems = visibleSettingsNavItems();
        focusSettingsNavItem(visibleItems[visibleItems.length - 1], true);
      }
    }

    function applySettingsFilter() {
      const query = normalizeSettingsFilter(settingsFilter?.value);
      const tokens = settingsFilterTokens(query);
      let visibleCount = 0;
      settingsSections.forEach((section) => {
        const isVisible = !tokens.length || settingsSectionMatches(section, tokens);
        section.hidden = !isVisible;
        section.setAttribute("aria-hidden", isVisible ? "false" : "true");
        if (isVisible) visibleCount += 1;
      });
      settingsNavItems.forEach((item) => {
        const section = document.getElementById(item.dataset.settingsNavTarget || "");
        const isFiltered = Boolean(tokens.length && section?.hidden);
        item.classList.toggle("settings-nav-filtered", isFiltered);
        item.setAttribute("aria-disabled", isFiltered ? "true" : "false");
        item.tabIndex = isFiltered ? -1 : 0;
      });
      setActiveSettingsNavItem(currentSettingsNavItem());
      settingsFilterClearButtons.forEach((button) => {
        button.hidden = !tokens.length;
      });
      scheduleSettingsNavFromScroll();
      if (settingsFilterEmpty) {
        settingsFilterEmpty.hidden = !tokens.length || visibleCount > 0;
      }
      if (settingsFilterStatus) {
        settingsFilterStatus.textContent = tokens.length
          ? visibleCount + " settings section" + (visibleCount === 1 ? "" : "s") + " matching " + tokens.join(" + ") + "."
          : "Showing all settings sections.";
      }
    }

    document.addEventListener("click", (event) => {
      const disabledNavItem = event.target.closest("[data-settings-nav-target][aria-disabled='true']");
      if (disabledNavItem) {
        event.preventDefault();
        settingsFilter?.focus();
        return;
      }
      const navItem = event.target.closest("[data-settings-nav-target]");
      if (navItem) {
        setActiveSettingsNavItem(navItem);
      }
      const clearFilter = event.target.closest("[data-settings-filter-clear]");
      if (clearFilter) {
        event.preventDefault();
        if (settingsFilter) {
          settingsFilter.value = "";
          settingsFilter.focus();
        }
        applySettingsFilter();
        return;
      }
      const action = event.target.closest("[data-action]");
      if (!action) return;
      event.preventDefault();
      vscode.postMessage({ type: action.dataset.action, scope: action.dataset.scope, key: action.dataset.key });
    });

    settingsNav?.addEventListener("keydown", handleSettingsNavKeydown);
    window.addEventListener("scroll", scheduleSettingsNavFromScroll, { passive: true });
    window.addEventListener("resize", scheduleSettingsNavFromScroll);
    settingsFilter?.addEventListener("input", applySettingsFilter);
    settingsFilter?.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !settingsFilter.value) return;
      event.preventDefault();
      settingsFilter.value = "";
      applySettingsFilter();
    });
    applySettingsFilter();
    updateSettingsNavFromScroll();
  </script>
</body>
</html>`;
  }
}

function settingRow(row: SettingsRow): string {
  return `
    <article class="settings-config-row settings-config-${row.tone}">
      <div>
        <h3>${escapeHtml(row.label)}</h3>
        <p>${escapeHtml(row.detail)}</p>
      </div>
      <span class="settings-row-status status status-${row.tone === "warn" ? "warning" : row.tone === "ok" ? "ready" : "new"}">${escapeHtml(row.tone === "warn" ? "Review" : row.tone === "ok" ? "Ready" : "Optional")}</span>
      <code>${escapeHtml(row.value)}</code>
      ${settingsActionButton({ type: "openSettings", key: row.key, label: "Edit", detail: `Edit ${row.label} in VS Code settings.` })}
    </article>
  `;
}

function settingsIssuePanel(issues: Array<{ label: string; detail: string; tone: "ok" | "warn"; action: SettingsAction }>): string {
  if (!issues.length) {
    return `
      <div class="settings-health-list settings-health-list-empty">
        <span class="status status-ready">Ready</span>
        <p>No settings issues are blocking CrabDB agent workflows.</p>
      </div>
    `;
  }
  return `
    <div class="settings-health-list" aria-label="Settings attention list">
      ${issues
        .slice(0, 6)
        .map(
          (issue) => `
            <article class="settings-health-item settings-health-item-${escapeClass(issue.tone)}">
              <div>
                <span class="status status-${issue.tone === "warn" ? "warning" : "ready"}">${issue.tone === "warn" ? "Review" : "Notice"}</span>
                <h3>${escapeHtml(issue.label)}</h3>
                <p>${escapeHtml(issue.detail)}</p>
              </div>
              ${settingsActionButton(issue.action)}
            </article>
          `
        )
        .join("")}
      ${issues.length > 6 ? `<p class="muted">Showing 6 of ${issues.length} settings findings.</p>` : ""}
    </div>
  `;
}

function settingsNextSteps(steps: SettingsNextStep[]): string {
  if (!steps.length) {
    return "";
  }
  return `
    <section class="settings-next-steps" aria-label="Recommended settings actions">
      <div class="settings-section-heading">
        <h2>Next steps</h2>
        <span class="settings-next-count">${steps.length} action${steps.length === 1 ? "" : "s"}</span>
      </div>
      <div class="settings-next-list">
        ${steps.map(settingsNextStep).join("")}
      </div>
    </section>
  `;
}

function settingsNextStep(step: SettingsNextStep): string {
  return `
    <article class="settings-next-step settings-next-step-${escapeClass(step.tone)}">
      <div>
        <span class="status status-${step.tone === "warn" ? "warning" : step.tone === "ok" ? "ready" : "new"}">${escapeHtml(step.tone === "warn" ? "Review" : step.tone === "ok" ? "Ready" : "Optional")}</span>
        <h3>${escapeHtml(step.label)}</h3>
        <p>${escapeHtml(step.detail)}</p>
      </div>
      ${settingsActionButton(step.action, step.tone === "warn")}
    </article>
  `;
}

function settingsActionButton(action: SettingsAction, primary = false): string {
  const classes = ["settings-action-button", `settings-action-${escapeClass(action.type)}`, primary ? "primary settings-primary-action" : ""]
    .filter(Boolean)
    .join(" ");
  return `
    <button type="button" class="${escapeHtml(classes)}" data-action="${escapeHtml(action.type)}"${action.scope ? ` data-scope="${escapeHtml(action.scope)}"` : ""}${action.key ? ` data-key="${escapeHtml(action.key)}"` : ""} title="${escapeHtml(action.detail)}" aria-label="${escapeHtml(`${action.label}. ${action.detail}`)}">
      ${settingsActionIcon(action.type)}
      <span>${escapeHtml(action.label)}</span>
    </button>
  `;
}

function settingsActionIcon(type: SettingsAction["type"]): string {
  const open = `<svg class="settings-action-icon" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">`;
  switch (type) {
    case "doctor":
      return `${open}<path d="M10 3l7 13H3L10 3z"/><path d="M10 8v4"/><path d="M10 15h.01"/></svg>`;
    case "startDaemon":
      return `${open}<path d="M4 6l4 4-4 4"/><path d="M10 14h6"/><path d="M11 6h5"/></svg>`;
    case "customProviders":
      return `${open}<path d="M7 8V5a3 3 0 0 1 6 0v3"/><rect x="5" y="8" width="10" height="8" rx="2"/><path d="M10 11v2"/></svg>`;
    default:
      return `${open}<path d="M5 6h10"/><path d="M7 10h6"/><path d="M9 14h2"/><circle cx="5" cy="6" r="1"/><circle cx="7" cy="10" r="1"/><circle cx="9" cy="14" r="1"/></svg>`;
  }
}

function providerCard(provider: ProviderCapabilityView): string {
  const command = redactString(provider.command);
  return `
    <article class="provider-card provider-card-${escapeClass(provider.tone)}">
      <header>
        <div>
          <h3>${escapeHtml(provider.label)}</h3>
          <p>${escapeHtml(provider.id)}</p>
        </div>
        <div class="provider-badges">
          ${provider.badges.map((badge) => `<span class="status status-${badge.tone === "warn" ? "warning" : badge.tone === "ok" ? "ready" : "new"}">${escapeHtml(badge.label)}</span>`).join("")}
        </div>
      </header>
      <p class="provider-detail">${escapeHtml(provider.detail)}</p>
      <dl class="settings-facts compact">
        <div><dt>Task names</dt><dd>${provider.supportsTaskName ? "supported" : "not advertised"}</dd></div>
        <div><dt>Checkpoint start</dt><dd>${provider.supportsFromRef ? "supported" : "not advertised"}</dd></div>
      </dl>
      <code class="provider-command">${escapeHtml(command)}</code>
    </article>
  `;
}

function providerRoutingPanel(summary: ProviderRoutingSummary): string {
  return `
    <article class="provider-routing provider-routing-${escapeClass(summary.tone)}">
      <div class="provider-routing-main">
        <span class="settings-kicker">Provider routing</span>
        <h3>${escapeHtml(summary.label)}</h3>
        <p>${escapeHtml(summary.detail)}</p>
      </div>
      <dl class="provider-routing-facts" aria-label="Provider routing facts">
        ${summary.facts.map(providerRoutingFact).join("")}
      </dl>
      <div class="provider-routing-action">
        ${settingsActionButton(summary.action, summary.tone === "warn")}
      </div>
    </article>
  `;
}

function providerRoutingFact(fact: ProviderRoutingFact): string {
  return `
    <div class="provider-routing-fact provider-routing-fact-${escapeClass(fact.tone)}">
      <dt>${escapeHtml(fact.label)}</dt>
      <dd>${escapeHtml(fact.value)}</dd>
    </div>
  `;
}

function providerMatrixRow(provider: ProviderCapabilityView): string {
  return `
    <div class="provider-matrix-row" role="row">
      <span role="cell">${escapeHtml(provider.label)}</span>
      ${capabilityCell(provider.crabdbBacked, provider.crabdbBacked ? "Durable" : "Raw ACP")}
      ${capabilityCell(provider.supportsTaskName, provider.supportsTaskName ? "Supported" : "No signal")}
      ${capabilityCell(provider.supportsFromRef, provider.supportsFromRef ? "Supported" : "No signal")}
    </div>
  `;
}

function capabilityCell(enabled: boolean, label: string): string {
  return `<span class="capability-cell ${enabled ? "on" : "off"}" role="cell">${escapeHtml(label)}</span>`;
}

function settingsSectionAttrs(section: SettingsSectionView | undefined): string {
  if (!section) {
    return "";
  }
  return `id="${escapeHtml(section.id)}" data-settings-section data-settings-section-id="${escapeHtml(section.id)}" data-settings-search="${escapeHtml([section.label, section.detail, section.searchText].join(" "))}"`;
}

function settingsNavItem(section: SettingsSectionView, active = false): string {
  return `
    <a class="settings-nav-${escapeClass(section.tone)} ${active ? "settings-nav-active" : ""}" href="#${escapeHtml(section.id)}" title="${escapeHtml(section.detail)}" data-settings-nav-target="${escapeHtml(section.id)}"${active ? ` aria-current="page"` : ""}>
      ${settingsIcon(section.icon)}
      <span class="settings-nav-label">${escapeHtml(section.label)}</span>
      ${section.badge ? `<span class="settings-nav-badge">${escapeHtml(section.badge)}</span>` : ""}
    </a>
  `;
}

function settingsIcon(icon: SettingsSectionIcon): string {
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

function metricCard(metric: SettingsMetric): string {
  return `
    <article class="settings-metric settings-metric-${escapeClass(metric.tone)}">
      <span>${escapeHtml(metric.label)}</span>
      <strong>${escapeHtml(metric.value)}</strong>
      <small>${escapeHtml(metric.detail)}</small>
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
