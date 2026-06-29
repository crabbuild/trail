import * as vscode from "vscode";
import { getExtensionConfig, type CustomProviderConfig } from "../config";

export interface AcpProviderProfile {
  id: string;
  label: string;
  command: string;
  args: string[];
  crabdbBacked: boolean;
  supportsTaskName: boolean;
  supportsFromRef: boolean;
}

export class ProviderRegistry {
  constructor(private readonly workspaceRoot: string) {}

  profiles(): AcpProviderProfile[] {
    const config = getExtensionConfig();
    const builtins: AcpProviderProfile[] = [
      this.crabDbProviderProfile("claude-code", "Claude Code via CrabDB"),
      this.crabDbProviderProfile("codex", "Codex via CrabDB")
    ];

    const custom = config.customProviders.map((provider) => this.customProfile(provider));
    return [...builtins, ...custom];
  }

  defaultProfile(): AcpProviderProfile {
    const config = getExtensionConfig();
    return (
      this.profiles().find((profile) => profile.id === config.defaultProvider) ??
      this.profiles()[0] ??
      this.crabDbProviderProfile("claude-code", "Claude Code via CrabDB")
    );
  }

  async pickProfile(): Promise<AcpProviderProfile | undefined> {
    const profiles = this.profiles();
    const picked = await vscode.window.showQuickPick(
      profiles.map((profile) => ({
        label: profile.label,
        description: profile.id,
        detail: profile.crabdbBacked
          ? "Runs through CrabDB so transcripts, checkpoints, and review state are durable."
          : "Custom ACP command. Use a CrabDB relay command to keep CrabDB as source of truth.",
        profile
      })),
      {
        title: "Choose ACP agent provider",
        placeHolder: "Provider for this CrabDB agent task"
      }
    );
    return picked?.profile;
  }

  private customProfile(provider: CustomProviderConfig): AcpProviderProfile {
    const command = expandVariables(provider.command, this.workspaceRoot);
    const args = (provider.args ?? []).map((arg) => expandVariables(arg, this.workspaceRoot));
    const crabdbBacked = command.includes("crabdb") || args.some((arg) => arg.includes("crabdb"));
    const crabdbAgentAcp = crabdbBacked && args.includes("agent") && args.includes("acp");
    return {
      id: provider.id,
      label: provider.label,
      command,
      args,
      crabdbBacked,
      supportsTaskName: crabdbAgentAcp,
      supportsFromRef: crabdbAgentAcp
    };
  }

  private crabDbProviderProfile(id: string, label: string): AcpProviderProfile {
    const config = getExtensionConfig();
    return {
      id,
      label,
      command: config.crabdbPath,
      args: ["--workspace", this.workspaceRoot, "agent", "acp", "--provider", id],
      crabdbBacked: true,
      supportsTaskName: true,
      supportsFromRef: true
    };
  }
}

function expandVariables(value: string, workspaceRoot: string): string {
  const config = getExtensionConfig();
  return value
    .replaceAll("${workspaceFolder}", workspaceRoot)
    .replaceAll("${workspaceRoot}", workspaceRoot)
    .replaceAll("${crabdbPath}", config.crabdbPath);
}
