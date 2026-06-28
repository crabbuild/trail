import assert from "node:assert/strict";
import test from "node:test";
import type { AcpProviderProfile } from "../acp/ProviderRegistry";
import type { ExtensionConfig } from "../config";
import { buildSettingsViewModel } from "../views/settingsModel";

const durableProvider: AcpProviderProfile = {
  id: "claude-code",
  label: "Claude Code via CrabDB",
  command: "crabdb",
  args: ["--workspace", "/repo", "agent", "acp", "--provider", "claude-code"],
  crabdbBacked: true,
  supportsTaskName: true,
  supportsFromRef: true
};

const rawProvider: AcpProviderProfile = {
  id: "raw-provider",
  label: "Raw ACP Provider",
  command: "raw-acp",
  args: ["--serve"],
  crabdbBacked: false,
  supportsTaskName: false,
  supportsFromRef: false
};

const baseConfig: ExtensionConfig = {
  crabdbPath: "crabdb",
  defaultProvider: "claude-code",
  autoStartDaemon: true,
  customProviders: []
};

test("builds a healthy settings model for a durable default provider", () => {
  const model = buildSettingsViewModel(baseConfig, [durableProvider]);

  assert.equal(model.healthTone, "healthy");
  assert.equal(model.healthLabel, "Ready");
  assert.equal(model.primaryAction.type, "doctor");
  assert.equal(model.nextSteps[0]?.label, "Run doctor");
  assert.equal(model.nextSteps.some((step) => step.label === "Workspace settings"), true);
  assert.equal(new Set(model.nextSteps.map((step) => `${step.action.type}:${step.action.key || ""}:${step.action.scope || ""}`)).size, model.nextSteps.length);
  assert.equal(model.providerRouting.tone, "ok");
  assert.equal(model.providerRouting.action.key, "crabdb.defaultProvider");
  assert.equal(model.providerRouting.facts.find((fact) => fact.label === "Durable")?.value, "1/1");
  assert.equal(model.providerCoverage.durable, 1);
  assert.equal(model.sections.find((section) => section.id === "providers")?.tone, "ok");
  assert.equal(model.sections.find((section) => section.id === "checkpoints")?.tone, "ok");
  assert.equal(model.rows.find((row) => row.key === "crabdb.defaultProvider")?.tone, "ok");
  assert.match(model.sections.find((section) => section.id === "overview")?.searchText || "", /doctor daemon/);
  assert.match(model.sections.find((section) => section.id === "display")?.searchText || "", /Shiki tokenization/);
});

test("flags raw default providers as attention states", () => {
  const model = buildSettingsViewModel(
    {
      ...baseConfig,
      defaultProvider: rawProvider.id
    },
    [durableProvider, rawProvider]
  );

  assert.equal(model.healthTone, "attention");
  assert.equal(model.primaryAction.key, "crabdb.defaultProvider");
  assert.equal(model.nextSteps[0]?.label, "Use durable provider");
  assert.equal(model.nextSteps[0]?.tone, "warn");
  assert.equal(model.nextSteps.some((step) => step.label === "Run doctor"), true);
  assert.equal(model.providerRouting.tone, "warn");
  assert.equal(model.providerRouting.label, "Default route is raw ACP");
  assert.equal(model.providerRouting.action.label, "Use durable route");
  assert.match(model.issues[0]?.label || "", /not durable/);
  assert.equal(model.providers.find((provider) => provider.id === rawProvider.id)?.tone, "warn");
  assert.equal(model.sections.find((section) => section.id === "providers")?.tone, "warn");
  assert.equal(model.sections.find((section) => section.id === "checkpoints")?.tone, "ok");
  assert.equal(model.sections.find((section) => section.id === "configuration")?.badge, "1");
  assert.match(model.sections.find((section) => section.id === "providers")?.searchText || "", /Raw ACP Provider/);
  assert.match(model.sections.find((section) => section.id === "providers")?.searchText || "", /raw-acp --serve/);
});

test("flags checkpoint navigation when no provider can start from a checkpoint", () => {
  const model = buildSettingsViewModel(
    {
      ...baseConfig,
      defaultProvider: rawProvider.id
    },
    [rawProvider]
  );

  assert.equal(model.sections.find((section) => section.id === "checkpoints")?.tone, "warn");
  assert.equal(model.sections.find((section) => section.id === "checkpoints")?.badge, "Review");
  assert.match(model.sections.find((section) => section.id === "checkpoints")?.detail || "", /No provider/);
});

test("surfaces missing default provider and empty CLI path", () => {
  const model = buildSettingsViewModel(
    {
      ...baseConfig,
      crabdbPath: "",
      defaultProvider: "missing-provider"
    },
    [durableProvider]
  );

  assert.equal(model.healthTone, "attention");
  assert.equal(model.primaryAction.key, "crabdb.path");
  assert.equal(model.nextSteps[0]?.label, "Set CLI path");
  assert.equal(model.nextSteps[1]?.label, "Choose provider");
  assert.equal(model.nextSteps.length, 4);
  assert.equal(model.providerRouting.tone, "warn");
  assert.equal(model.providerRouting.label, "Default provider is unavailable");
  assert.equal(model.providerRouting.facts.find((fact) => fact.label === "Default")?.tone, "warn");
  assert.equal(model.issues.some((issue) => issue.label === "Default provider is unavailable"), true);
  assert.equal(model.rows.find((row) => row.key === "crabdb.path")?.tone, "warn");
  assert.equal(model.sections.find((section) => section.id === "overview")?.badge, "2");
  assert.equal(model.sections.find((section) => section.id === "providers")?.badge, "Review");
  assert.match(model.sections.find((section) => section.id === "configuration")?.searchText || "", /crabdb.path/);
  assert.match(model.sections.find((section) => section.id === "diagnostics")?.searchText || "", /missing-provider/);
});

test("promotes provider creation when no providers exist", () => {
  const model = buildSettingsViewModel(
    {
      ...baseConfig,
      defaultProvider: ""
    },
    []
  );

  assert.equal(model.providerRouting.label, "No provider route configured");
  assert.equal(model.providerRouting.action.type, "customProviders");
  assert.equal(model.primaryAction.type, "customProviders");
  assert.equal(model.nextSteps[0]?.label, "Add provider");
  assert.equal(model.nextSteps[0]?.action.type, "customProviders");
  assert.equal(model.providerRouting.facts.find((fact) => fact.label === "Durable")?.value, "0/1");
  assert.equal(model.sections.find((section) => section.id === "providers")?.tone, "warn");
});

test("tracks custom and long provider capability coverage", () => {
  const longProvider: AcpProviderProfile = {
    ...rawProvider,
    id: "provider-with-a-very-long-identifier-that-should-wrap-in-the-settings-ui",
    label: "Provider With A Very Long Human Readable Name For Layout Hardening"
  };
  const model = buildSettingsViewModel(
    {
      ...baseConfig,
      customProviders: [
        {
          id: longProvider.id,
          label: longProvider.label,
          command: "raw-acp"
        }
      ]
    },
    [durableProvider, longProvider]
  );

  assert.equal(model.providerCoverage.total, 2);
  assert.equal(model.providerCoverage.custom, 1);
  assert.equal(model.providerCoverage.raw, 1);
  assert.equal(model.metrics.find((metric) => metric.label === "Provider durability")?.tone, "ok");
  assert.equal(model.providers[1]?.badges.some((badge) => badge.label === "Raw ACP"), true);
  assert.equal(model.sections.map((section) => section.id).includes("diagnostics"), true);
  assert.match(model.sections.find((section) => section.id === "providers")?.searchText || "", /Provider With A Very Long Human Readable Name/);
  assert.match(model.sections.find((section) => section.id === "checkpoints")?.searchText || "", /no checkpoint start/);
});
