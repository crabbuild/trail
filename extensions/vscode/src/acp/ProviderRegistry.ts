import * as vscode from "vscode";
import { getExtensionConfig, type CustomProviderConfig } from "../config";

export interface AcpProviderProfile {
  id: string;
  label: string;
  command: string;
  args: string[];
  trailBacked: boolean;
  supportsTaskName: boolean;
  supportsFromRef: boolean;
}

export class ProviderRegistry {
  constructor(private readonly workspaceRoot: string) {}

  profiles(): AcpProviderProfile[] {
    const config = getExtensionConfig();
    const builtins: AcpProviderProfile[] = [
      this.trailProviderProfile("claude-code", "Claude Code via Trail"),
      this.trailProviderProfile("codex", "Codex via Trail")
    ];

    const custom = config.customProviders.map((provider) => this.customProfile(provider));
    return [...builtins, ...custom];
  }

  defaultProfile(): AcpProviderProfile {
    const config = getExtensionConfig();
    return (
      this.profiles().find((profile) => profile.id === config.defaultProvider) ??
      this.profiles()[0] ??
      this.trailProviderProfile("claude-code", "Claude Code via Trail")
    );
  }

  async pickProfile(): Promise<AcpProviderProfile | undefined> {
    const profiles = this.profiles();
    const picked = await vscode.window.showQuickPick(
      profiles.map((profile) => ({
        label: profile.label,
        description: profile.id,
        detail: profile.trailBacked
          ? "Runs through Trail so transcripts, checkpoints, and review state are durable."
          : "Custom ACP command. Use a Trail relay command to keep Trail as source of truth.",
        profile
      })),
      {
        title: "Choose ACP agent provider",
        placeHolder: "Provider for this Trail agent task"
      }
    );
    return picked?.profile;
  }

  private customProfile(provider: CustomProviderConfig): AcpProviderProfile {
    const command = expandVariables(provider.command, this.workspaceRoot);
    const args = (provider.args ?? []).map((arg) => expandVariables(arg, this.workspaceRoot));
    const trailBacked = command.includes("trail") || args.some((arg) => arg.includes("trail"));
    const trailAgentAcp = trailBacked && args.includes("agent") && args.includes("acp");
    return {
      id: provider.id,
      label: provider.label,
      command,
      args,
      trailBacked,
      supportsTaskName: trailAgentAcp,
      supportsFromRef: trailAgentAcp
    };
  }

  private trailProviderProfile(id: string, label: string): AcpProviderProfile {
    const config = getExtensionConfig();
    return {
      id,
      label,
      command: config.trailPath,
      args: ["--workspace", this.workspaceRoot, "agent", "acp", "--provider", id],
      trailBacked: true,
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
    .replaceAll("${trailPath}", config.trailPath);
}
