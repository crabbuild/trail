import type { JsonObject } from "../shared/acpTypes";

export interface PromptCapabilities {
  image: boolean;
  audio: boolean;
  embeddedContext: boolean;
}

export interface McpCapabilities {
  http: boolean;
  sse: boolean;
  [key: string]: unknown;
}

export interface AgentCapabilities {
  loadSession: boolean;
  promptCapabilities: PromptCapabilities;
  mcpCapabilities: McpCapabilities;
  sessionCapabilities: JsonObject;
  auth: JsonObject;
  raw: JsonObject;
}

export function defaultAgentCapabilities(): AgentCapabilities {
  return {
    loadSession: false,
    promptCapabilities: {
      image: false,
      audio: false,
      embeddedContext: false
    },
    mcpCapabilities: {
      http: false,
      sse: false
    },
    sessionCapabilities: {},
    auth: {},
    raw: {}
  };
}

export function capabilitiesFromInitializeResponse(response: unknown): AgentCapabilities {
  return normalizeAgentCapabilities(asRecord(response).agentCapabilities);
}

export function normalizeAgentCapabilities(value: unknown): AgentCapabilities {
  const defaults = defaultAgentCapabilities();
  const record = asRecord(value);
  const prompt = asRecord(record.promptCapabilities);
  const mcp = asRecord(record.mcpCapabilities);
  return {
    loadSession: record.loadSession === true,
    promptCapabilities: {
      image: prompt.image === true,
      audio: prompt.audio === true,
      embeddedContext: prompt.embeddedContext === true
    },
    mcpCapabilities: {
      ...defaults.mcpCapabilities,
      ...mcp,
      http: mcp.http === true,
      sse: mcp.sse === true
    },
    sessionCapabilities: asRecord(record.sessionCapabilities),
    auth: asRecord(record.auth),
    raw: record
  };
}

function asRecord(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonObject) : {};
}
