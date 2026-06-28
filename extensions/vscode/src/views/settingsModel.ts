import type { AcpProviderProfile } from "../acp/ProviderRegistry";
import type { ExtensionConfig } from "../config";

export type SettingsTone = "ok" | "warn" | "neutral";
export type SettingsHealthTone = "healthy" | "attention" | "limited";
export type SettingsSectionIcon = "diagnostics" | "display" | "file" | "history" | "provider" | "review" | "terminal";
export type SettingsSectionId =
  | "overview"
  | "providers"
  | "configuration"
  | "agent-behavior"
  | "context"
  | "review"
  | "checkpoints"
  | "display"
  | "diagnostics";

export interface SettingsAction {
  label: string;
  detail: string;
  type: "openSettings" | "doctor" | "startDaemon" | "customProviders";
  key?: string | undefined;
  scope?: "workspace" | "user" | undefined;
}

export interface SettingsMetric {
  label: string;
  value: string;
  detail: string;
  tone: SettingsTone;
}

export interface SettingsIssue {
  label: string;
  detail: string;
  tone: Exclude<SettingsTone, "neutral">;
  action: SettingsAction;
}

export interface SettingsNextStep {
  label: string;
  detail: string;
  tone: SettingsTone;
  action: SettingsAction;
}

export interface SettingsRow {
  key: string;
  label: string;
  value: string;
  detail: string;
  tone: SettingsTone;
}

export interface ProviderCapabilityView {
  id: string;
  label: string;
  detail: string;
  command: string;
  isDefault: boolean;
  crabdbBacked: boolean;
  supportsTaskName: boolean;
  supportsFromRef: boolean;
  tone: SettingsTone;
  badges: Array<{ label: string; tone: SettingsTone }>;
}

export interface ProviderRoutingFact {
  label: string;
  value: string;
  tone: SettingsTone;
}

export interface ProviderRoutingSummary {
  tone: SettingsTone;
  label: string;
  detail: string;
  action: SettingsAction;
  facts: ProviderRoutingFact[];
}

export interface SettingsSectionView {
  id: SettingsSectionId;
  label: string;
  detail: string;
  icon: SettingsSectionIcon;
  tone: SettingsTone;
  searchText: string;
  badge?: string | undefined;
}

export interface SettingsViewModel {
  defaultProvider?: AcpProviderProfile | undefined;
  healthTone: SettingsHealthTone;
  healthLabel: string;
  healthDetail: string;
  primaryAction: SettingsAction;
  secondaryActions: SettingsAction[];
  nextSteps: SettingsNextStep[];
  metrics: SettingsMetric[];
  issues: SettingsIssue[];
  rows: SettingsRow[];
  providers: ProviderCapabilityView[];
  providerRouting: ProviderRoutingSummary;
  sections: SettingsSectionView[];
  providerCoverage: {
    total: number;
    durable: number;
    raw: number;
    taskNames: number;
    checkpoints: number;
    custom: number;
  };
}

export function buildSettingsViewModel(config: ExtensionConfig, providers: AcpProviderProfile[]): SettingsViewModel {
  const configuredDefaultProvider = providers.find((provider) => provider.id === config.defaultProvider);
  const defaultProvider = configuredDefaultProvider ?? providers[0];
  const coverage = providerCoverage(config, providers);
  const issues = settingsIssues(config, providers, configuredDefaultProvider);
  const rows = settingsRows(config, configuredDefaultProvider);
  const healthTone = settingsHealthTone(issues, defaultProvider, coverage);
  const providerViews = providers.map((provider) => providerCapabilityView(provider, defaultProvider?.id || config.defaultProvider));
  const providerRouting = providerRoutingSummary(config, providers, configuredDefaultProvider, coverage);
  const secondaryActions = [
    { type: "doctor", label: "Run doctor", detail: "Check the local CrabDB toolchain." },
    { type: "startDaemon", label: "Start daemon", detail: "Bring up CrabDB services for review and queue state." },
    { type: "openSettings", scope: "workspace", label: "Workspace settings", detail: "Edit CrabDB settings for this repository." },
    { type: "openSettings", scope: "user", label: "User settings", detail: "Edit global CrabDB defaults." }
  ] satisfies SettingsAction[];
  const primaryAction = primarySettingsAction(issues, config);
  return {
    defaultProvider,
    healthTone,
    healthLabel: settingsHealthLabel(healthTone),
    healthDetail: settingsHealthDetail(healthTone, defaultProvider, config.defaultProvider),
    primaryAction,
    secondaryActions,
    nextSteps: settingsNextSteps(issues, providerRouting, primaryAction, secondaryActions),
    metrics: settingsMetrics(config, providers, configuredDefaultProvider, coverage),
    issues,
    rows,
    providers: providerViews,
    providerRouting,
    sections: settingsSections(config, providers, providerViews, configuredDefaultProvider, coverage, issues, rows, healthTone),
    providerCoverage: coverage
  };
}

function settingsNextSteps(
  issues: SettingsIssue[],
  providerRouting: ProviderRoutingSummary,
  primaryAction: SettingsAction,
  secondaryActions: SettingsAction[]
): SettingsNextStep[] {
  const steps: SettingsNextStep[] = [];
  for (const issue of issues) {
    steps.push({
      label: issue.action.label,
      detail: issue.detail,
      tone: issue.tone,
      action: issue.action
    });
  }
  if (providerRouting.tone === "warn") {
    steps.push({
      label: providerRouting.action.label,
      detail: providerRouting.detail,
      tone: "warn",
      action: providerRouting.action
    });
  }
  steps.push({
    label: primaryAction.label,
    detail: primaryAction.detail,
    tone: issues.some((issue) => issue.tone === "warn") ? "warn" : "ok",
    action: primaryAction
  });
  for (const action of secondaryActions) {
    steps.push({
      label: action.label,
      detail: action.detail,
      tone: action.type === "startDaemon" ? "neutral" : "ok",
      action
    });
  }
  return uniqueSettingsSteps(steps).slice(0, 4);
}

function uniqueSettingsSteps(steps: SettingsNextStep[]): SettingsNextStep[] {
  const seen = new Set<string>();
  return steps.filter((step) => {
    const key = `${step.action.type}:${step.action.key || ""}:${step.action.scope || ""}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function providerRoutingSummary(
  config: ExtensionConfig,
  providers: AcpProviderProfile[],
  configuredDefaultProvider: AcpProviderProfile | undefined,
  coverage: SettingsViewModel["providerCoverage"]
): ProviderRoutingSummary {
  if (!providers.length) {
    return {
      tone: "warn",
      label: "No provider route configured",
      detail: "Add a built-in or custom ACP route before starting an agent task.",
      action: {
        type: "customProviders",
        label: "Add provider",
        detail: "Open custom provider configuration."
      },
      facts: providerRoutingFacts(config, configuredDefaultProvider, coverage)
    };
  }
  if (!configuredDefaultProvider) {
    return {
      tone: "warn",
      label: "Default provider is unavailable",
      detail: "Choose one of the configured providers so new tasks start predictably.",
      action: editSettingAction("crabdb.defaultProvider", "Choose provider", "Select an available ACP provider."),
      facts: providerRoutingFacts(config, configuredDefaultProvider, coverage)
    };
  }
  if (!configuredDefaultProvider.crabdbBacked) {
    return {
      tone: "warn",
      label: "Default route is raw ACP",
      detail: "Raw providers can run, but durable transcript, checkpoint, and review state need a CrabDB relay route.",
      action: editSettingAction("crabdb.defaultProvider", "Use durable route", "Route the default provider through CrabDB."),
      facts: providerRoutingFacts(config, configuredDefaultProvider, coverage)
    };
  }
  return {
    tone: coverage.checkpoints ? "ok" : "neutral",
    label: "Default route is durable",
    detail: coverage.checkpoints
      ? "New tasks can start through CrabDB and recover from checkpoints."
      : "The default provider is durable; checkpoint-start support is not advertised by any provider.",
    action: editSettingAction("crabdb.defaultProvider", "Change default", "Choose a different default provider."),
    facts: providerRoutingFacts(config, configuredDefaultProvider, coverage)
  };
}

function providerRoutingFacts(
  config: ExtensionConfig,
  configuredDefaultProvider: AcpProviderProfile | undefined,
  coverage: SettingsViewModel["providerCoverage"]
): ProviderRoutingFact[] {
  const total = Math.max(coverage.total, 1);
  return [
    {
      label: "Default",
      value: configuredDefaultProvider?.label || config.defaultProvider || "Missing",
      tone: configuredDefaultProvider?.crabdbBacked ? "ok" : "warn"
    },
    {
      label: "Durable",
      value: `${coverage.durable}/${total}`,
      tone: coverage.durable ? "ok" : "warn"
    },
    {
      label: "Checkpoint start",
      value: `${coverage.checkpoints}/${total}`,
      tone: coverage.checkpoints ? "ok" : coverage.total ? "neutral" : "warn"
    },
    {
      label: "Custom",
      value: String(coverage.custom),
      tone: coverage.custom ? "ok" : "neutral"
    }
  ];
}

function providerCoverage(config: ExtensionConfig, providers: AcpProviderProfile[]): SettingsViewModel["providerCoverage"] {
  return {
    total: providers.length,
    durable: providers.filter((provider) => provider.crabdbBacked).length,
    raw: providers.filter((provider) => !provider.crabdbBacked).length,
    taskNames: providers.filter((provider) => provider.supportsTaskName).length,
    checkpoints: providers.filter((provider) => provider.supportsFromRef).length,
    custom: config.customProviders.length
  };
}

function settingsHealthTone(
  issues: SettingsIssue[],
  defaultProvider: AcpProviderProfile | undefined,
  coverage: SettingsViewModel["providerCoverage"]
): SettingsHealthTone {
  if (!defaultProvider || issues.some((issue) => issue.tone === "warn")) {
    return "attention";
  }
  if (!coverage.total || coverage.durable === 0) {
    return "limited";
  }
  return "healthy";
}

function settingsHealthLabel(tone: SettingsHealthTone): string {
  switch (tone) {
    case "healthy":
      return "Ready";
    case "limited":
      return "Limited";
    default:
      return "Needs attention";
  }
}

function settingsHealthDetail(
  tone: SettingsHealthTone,
  defaultProvider: AcpProviderProfile | undefined,
  configuredProviderId: string
): string {
  if (!defaultProvider) {
    return `Default provider ${configuredProviderId || "provider"} is not available. Update provider routing before starting a task.`;
  }
  if (tone === "healthy") {
    return `${defaultProvider.label} is routed through CrabDB with durable transcript, checkpoint, review, and merge state.`;
  }
  if (tone === "limited") {
    return "Provider routing can start tasks, but durability and checkpoint coverage need review.";
  }
  return "Review the highlighted settings before relying on CrabDB coordination for production work.";
}

function primarySettingsAction(issues: SettingsIssue[], config: ExtensionConfig): SettingsAction {
  const firstIssue = issues[0];
  if (firstIssue) {
    return firstIssue.action;
  }
  if (!config.autoStartDaemon) {
    return {
      type: "startDaemon",
      label: "Start daemon",
      detail: "Run CrabDB services now."
    };
  }
  return {
    type: "doctor",
    label: "Run doctor",
    detail: "Verify the current CrabDB setup."
  };
}

function settingsIssues(
  config: ExtensionConfig,
  providers: AcpProviderProfile[],
  configuredDefaultProvider: AcpProviderProfile | undefined
): SettingsIssue[] {
  const issues: SettingsIssue[] = [];
  if (!providers.length) {
    issues.push({
      label: "No providers are configured",
      detail: "Add a built-in or custom ACP provider before starting an agent task.",
      tone: "warn",
      action: {
        type: "customProviders",
        label: "Add provider",
        detail: "Open custom provider configuration."
      }
    });
  }
  if (!config.crabdbPath.trim()) {
    issues.push({
      label: "CrabDB CLI path is empty",
      detail: "Set the executable path so daemon, review, queue, and ACP relay commands can run.",
      tone: "warn",
      action: editSettingAction("crabdb.path", "Set CLI path", "Choose the CrabDB executable.")
    });
  }
  if (!configuredDefaultProvider && providers.length) {
    issues.push({
      label: "Default provider is unavailable",
      detail: `${config.defaultProvider || "The configured provider"} does not match a known built-in or custom provider.`,
      tone: "warn",
      action: editSettingAction("crabdb.defaultProvider", "Choose provider", "Select an available ACP provider.")
    });
  } else if (configuredDefaultProvider && !configuredDefaultProvider.crabdbBacked) {
    issues.push({
      label: "Default provider is not durable",
      detail: "Raw ACP providers can run, but CrabDB cannot guarantee checkpoint and review state unless the command relays through CrabDB.",
      tone: "warn",
      action: editSettingAction("crabdb.defaultProvider", "Use durable provider", "Route the default provider through CrabDB.")
    });
  }
  if (!config.autoStartDaemon) {
    issues.push({
      label: "Daemon auto-start is off",
      detail: "Manual daemon startup is fine for controlled workspaces, but review and queue features need the service available.",
      tone: "ok",
      action: editSettingAction("crabdb.autoStartDaemon", "Review daemon setting", "Decide how CrabDB services should start.")
    });
  }
  return issues;
}

function settingsMetrics(
  config: ExtensionConfig,
  providers: AcpProviderProfile[],
  configuredDefaultProvider: AcpProviderProfile | undefined,
  coverage: SettingsViewModel["providerCoverage"]
): SettingsMetric[] {
  return [
    {
      label: "Provider durability",
      value: `${coverage.durable}/${Math.max(coverage.total, 1)} CrabDB-backed`,
      detail: coverage.raw ? `${coverage.raw} raw ACP provider${coverage.raw === 1 ? "" : "s"} need caution.` : "All providers are durable.",
      tone: coverage.durable ? "ok" : "warn"
    },
    {
      label: "Default provider",
      value: configuredDefaultProvider?.id || config.defaultProvider || "missing",
      detail: configuredDefaultProvider?.label || "No matching provider profile was found.",
      tone: configuredDefaultProvider?.crabdbBacked ? "ok" : "warn"
    },
    {
      label: "Daemon",
      value: config.autoStartDaemon ? "Auto-starts" : "Manual start",
      detail: config.autoStartDaemon ? "The extension starts CrabDB services when needed." : "Start the daemon before queue or review work.",
      tone: config.autoStartDaemon ? "ok" : "neutral"
    },
    {
      label: "Capability coverage",
      value: `${coverage.taskNames}/${Math.max(coverage.total, 1)} task names`,
      detail: `${coverage.checkpoints}/${Math.max(coverage.total, 1)} providers support checkpoint start.`,
      tone: coverage.taskNames && coverage.checkpoints ? "ok" : "neutral"
    }
  ];
}

function settingsRows(config: ExtensionConfig, configuredDefaultProvider: AcpProviderProfile | undefined): SettingsRow[] {
  return [
    {
      key: "crabdb.path",
      label: "CrabDB CLI",
      value: config.crabdbPath,
      detail: "Executable used for daemon, ACP provider relay, review, queue, and diagnostics commands.",
      tone: config.crabdbPath.trim() ? "ok" : "warn"
    },
    {
      key: "crabdb.defaultProvider",
      label: "Default provider",
      value: configuredDefaultProvider
        ? `${configuredDefaultProvider.label} (${configuredDefaultProvider.id})`
        : config.defaultProvider || "Not configured",
      detail: configuredDefaultProvider?.crabdbBacked
        ? "Default provider runs through CrabDB, so transcript, checkpoint, and review state stay durable."
        : "Default provider is not CrabDB-backed. Use a CrabDB relay command when durability matters.",
      tone: configuredDefaultProvider?.crabdbBacked ? "ok" : "warn"
    },
    {
      key: "crabdb.autoStartDaemon",
      label: "Daemon auto-start",
      value: config.autoStartDaemon ? "Enabled" : "Disabled",
      detail: config.autoStartDaemon
        ? "The extension starts CrabDB daemon when no endpoint is discovered."
        : "Start the daemon manually before agent review and queue features can use the daemon endpoint.",
      tone: config.autoStartDaemon ? "ok" : "neutral"
    },
    {
      key: "crabdb.customProviders",
      label: "Custom providers",
      value: `${config.customProviders.length} configured`,
      detail: "Custom ACP commands can be added for local tools, hosted gateways, or provider experiments.",
      tone: config.customProviders.length ? "ok" : "neutral"
    }
  ];
}

function providerCapabilityView(provider: AcpProviderProfile, defaultProviderId: string): ProviderCapabilityView {
  const isDefault = provider.id === defaultProviderId;
  return {
    id: provider.id,
    label: provider.label,
    detail: provider.crabdbBacked
      ? "Durable route with CrabDB transcript, checkpoint, review, and merge coordination."
      : "Raw ACP route. Use a CrabDB relay command when this provider needs durable state.",
    command: [provider.command, ...provider.args].join(" "),
    isDefault,
    crabdbBacked: provider.crabdbBacked,
    supportsTaskName: provider.supportsTaskName,
    supportsFromRef: provider.supportsFromRef,
    tone: provider.crabdbBacked ? "ok" : "warn",
    badges: [
      ...(isDefault ? [{ label: "Default", tone: "ok" as const }] : []),
      { label: provider.crabdbBacked ? "Durable" : "Raw ACP", tone: provider.crabdbBacked ? "ok" : "warn" },
      { label: provider.supportsTaskName ? "Task names" : "No task names", tone: provider.supportsTaskName ? "ok" : "neutral" },
      { label: provider.supportsFromRef ? "Checkpoint start" : "No checkpoint start", tone: provider.supportsFromRef ? "ok" : "neutral" }
    ]
  };
}

function settingsSections(
  config: ExtensionConfig,
  providers: AcpProviderProfile[],
  providerViews: ProviderCapabilityView[],
  configuredDefaultProvider: AcpProviderProfile | undefined,
  coverage: SettingsViewModel["providerCoverage"],
  issues: SettingsIssue[],
  rows: SettingsRow[],
  healthTone: SettingsHealthTone
): SettingsSectionView[] {
  const warnIssues = issues.filter((issue) => issue.tone === "warn").length;
  const warnRows = rows.filter((row) => row.tone === "warn").length;
  const providerNeedsReview = !providers.length || !configuredDefaultProvider || !configuredDefaultProvider.crabdbBacked;
  const checkpointNeedsReview = providers.length > 0 && coverage.checkpoints === 0;
  const healthDetail = settingsHealthDetail(healthTone, configuredDefaultProvider ?? providers[0], config.defaultProvider);
  return [
    {
      id: "overview",
      label: "Overview",
      detail: warnIssues ? `${warnIssues} setup issue${warnIssues === 1 ? "" : "s"} need review.` : "CrabDB workflow readiness at a glance.",
      icon: "display",
      tone: healthTone === "healthy" ? "ok" : healthTone === "attention" ? "warn" : "neutral",
      searchText: settingsSearchText(
        "overview readiness health control plane setup issues doctor daemon",
        settingsHealthLabel(healthTone),
        healthDetail,
        ...issues.flatMap((issue) => [issue.label, issue.detail, issue.action.label, issue.action.detail])
      ),
      badge: warnIssues ? String(warnIssues) : undefined
    },
    {
      id: "providers",
      label: "Providers",
      detail: providerNeedsReview
        ? "Provider routing needs attention before relying on durable task state."
        : `${coverage.durable} durable route${coverage.durable === 1 ? "" : "s"} configured.`,
      icon: "provider",
      tone: providerNeedsReview ? "warn" : "ok",
      searchText: settingsSearchText(
        "providers routing capability matrix default custom durable raw acp task names checkpoint start",
        `${coverage.total} configured providers`,
        `${coverage.durable} durable routes`,
        `${coverage.raw} raw ACP providers`,
        `${coverage.custom} custom providers`,
        ...providerViews.flatMap((provider) => [
          provider.id,
          provider.label,
          provider.detail,
          provider.command,
          provider.isDefault ? "default provider" : "",
          provider.crabdbBacked ? "CrabDB durable route" : "raw ACP route",
          provider.supportsTaskName ? "task names supported" : "no task names",
          provider.supportsFromRef ? "checkpoint start supported" : "no checkpoint start",
          ...provider.badges.map((badge) => badge.label)
        ])
      ),
      badge: providerNeedsReview ? "Review" : `${coverage.durable}/${coverage.total}`
    },
    {
      id: "configuration",
      label: "Configuration",
      detail: warnRows ? `${warnRows} setting${warnRows === 1 ? "" : "s"} need edits.` : "Core CrabDB settings are ready.",
      icon: "review",
      tone: warnRows ? "warn" : "ok",
      searchText: settingsSearchText(
        "configuration settings keys workspace user local global",
        ...rows.flatMap((row) => [row.key, row.label, row.value, row.detail, row.tone])
      ),
      badge: warnRows ? String(warnRows) : undefined
    },
    {
      id: "agent-behavior",
      label: "Agent behavior",
      detail: config.autoStartDaemon ? "Daemon startup and provider behavior are automatic." : "Daemon startup is manual for this workspace.",
      icon: "terminal",
      tone: config.autoStartDaemon ? "ok" : "neutral",
      searchText: settingsSearchText(
        "agent behavior session routing modes slash commands daemon auto-start provider sessions follow-up",
        config.autoStartDaemon ? "daemon auto-start enabled automatic" : "daemon auto-start disabled manual"
      ),
      badge: config.autoStartDaemon ? undefined : "Manual"
    },
    {
      id: "context",
      label: "Context",
      detail: "Composer attachment, resource, and redaction behavior.",
      icon: "file",
      tone: "ok",
      searchText: settingsSearchText(
        "context attachments composer selection active file diagnostics changed files terminal output history image audio embedded redaction"
      )
    },
    {
      id: "review",
      label: "Review",
      detail: coverage.durable ? "Review, queue, and coordination surfaces can use durable CrabDB state." : "Durable review state needs a CrabDB-backed provider.",
      icon: "review",
      tone: coverage.durable ? "ok" : "warn",
      searchText: settingsSearchText(
        "review safety drawer readiness changed paths conflicts gates transcript apply dry-run merge queue rewind preserve failed attempt coordination approvals tests evals",
        coverage.durable ? "durable review state available" : "durable review state missing"
      ),
      badge: coverage.durable ? undefined : "No durable"
    },
    {
      id: "checkpoints",
      label: "Checkpoints",
      detail: checkpointNeedsReview ? "No provider advertises checkpoint start support." : "Checkpoint-aware task start is available.",
      icon: "history",
      tone: checkpointNeedsReview ? "warn" : "ok",
      searchText: settingsSearchText(
        "checkpoints checkpoint start turn durable follow-up provider from ref rewind preserve failed attempt",
        checkpointNeedsReview ? "no checkpoint start support" : "checkpoint start supported",
        ...providerViews.map((provider) => `${provider.label} ${provider.supportsFromRef ? "supports checkpoint start" : "no checkpoint start"}`)
      ),
      badge: checkpointNeedsReview ? "Review" : undefined
    },
    {
      id: "display",
      label: "Display",
      detail: "Theme, composer, diff, code, and transcript rendering behavior.",
      icon: "display",
      tone: "ok",
      searchText: settingsSearchText(
        "display theme composer frame prompt border diff code previews Shiki tokenization transcript rendering raw JSON terminal output details high contrast"
      )
    },
    {
      id: "diagnostics",
      label: "Diagnostics",
      detail: "Workspace facts and configuration keys for support.",
      icon: "diagnostics",
      tone: "neutral",
      searchText: settingsSearchText(
        "diagnostics advanced configuration workspace facts support provider durability capability coverage configuration keys",
        config.defaultProvider,
        config.crabdbPath,
        config.autoStartDaemon ? "auto-start daemon enabled" : "auto-start daemon disabled",
        "crabdb.path crabdb.defaultProvider crabdb.autoStartDaemon crabdb.customProviders"
      )
    }
  ];
}

function settingsSearchText(...values: string[]): string {
  return values
    .map((value) => value.trim())
    .filter(Boolean)
    .join(" ");
}

function editSettingAction(key: string, label: string, detail: string): SettingsAction {
  return {
    type: "openSettings",
    key,
    label,
    detail
  };
}
