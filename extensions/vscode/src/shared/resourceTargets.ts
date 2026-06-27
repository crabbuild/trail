import path from "node:path";
import { fileURLToPath } from "node:url";

export type ResourceTarget =
  | {
      kind: "workspace-file";
      path: string;
      original: string;
    }
  | {
      kind: "external-file";
      path: string;
      original: string;
    }
  | {
      kind: "external-uri";
      uri: string;
      scheme: "http" | "https";
      original: string;
    }
  | {
      kind: "unsupported-uri";
      uri: string;
      scheme: string;
      original: string;
    }
  | {
      kind: "invalid";
      original: string;
      reason: string;
    };

export function classifyResourceTarget(value: string, workspaceRoot: string): ResourceTarget {
  const original = value;
  const input = value.trim();
  if (!input) {
    return { kind: "invalid", original, reason: "Empty resource target." };
  }

  const scheme = uriScheme(input);
  if (!scheme) {
    return classifyFilePath(input, workspaceRoot, original);
  }

  if (scheme === "file") {
    try {
      return classifyFilePath(fileURLToPath(input), workspaceRoot, original);
    } catch (error) {
      return {
        kind: "invalid",
        original,
        reason: error instanceof Error ? error.message : String(error)
      };
    }
  }

  if (scheme === "http" || scheme === "https") {
    return {
      kind: "external-uri",
      uri: input,
      scheme,
      original
    };
  }

  return {
    kind: "unsupported-uri",
    uri: input,
    scheme,
    original
  };
}

export function isPathInsideWorkspace(candidatePath: string, workspaceRoot: string): boolean {
  const relative = path.relative(path.resolve(workspaceRoot), path.resolve(candidatePath));
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function classifyFilePath(value: string, workspaceRoot: string, original: string): ResourceTarget {
  const candidatePath = path.resolve(path.isAbsolute(value) ? value : path.join(workspaceRoot, value));
  if (isPathInsideWorkspace(candidatePath, workspaceRoot)) {
    return {
      kind: "workspace-file",
      path: candidatePath,
      original
    };
  }
  return {
    kind: "external-file",
    path: candidatePath,
    original
  };
}

function uriScheme(value: string): string | undefined {
  const match = /^([a-z][a-z0-9+.-]*):/i.exec(value);
  return match?.[1]?.toLowerCase();
}
