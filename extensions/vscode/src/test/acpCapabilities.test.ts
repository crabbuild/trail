import assert from "node:assert/strict";
import test from "node:test";
import {
  capabilitiesFromInitializeResponse,
  defaultAgentCapabilities,
  normalizeAgentCapabilities
} from "../acp/AcpCapabilities";

test("uses conservative ACP capability defaults", () => {
  const capabilities = defaultAgentCapabilities();

  assert.equal(capabilities.loadSession, false);
  assert.equal(capabilities.promptCapabilities.image, false);
  assert.equal(capabilities.promptCapabilities.audio, false);
  assert.equal(capabilities.promptCapabilities.embeddedContext, false);
  assert.equal(capabilities.mcpCapabilities.http, false);
  assert.equal(capabilities.mcpCapabilities.sse, false);
});

test("normalizes prompt and session capabilities from initialize response", () => {
  const capabilities = capabilitiesFromInitializeResponse({
    protocolVersion: "1",
    agentCapabilities: {
      loadSession: true,
      promptCapabilities: {
        image: true,
        audio: false,
        embeddedContext: true
      },
      mcpCapabilities: {
        http: true
      },
      sessionCapabilities: {
        list: true,
        close: true
      },
      auth: {
        logout: true
      }
    }
  });

  assert.equal(capabilities.loadSession, true);
  assert.deepEqual(capabilities.promptCapabilities, {
    image: true,
    audio: false,
    embeddedContext: true
  });
  assert.equal(capabilities.mcpCapabilities.http, true);
  assert.equal(capabilities.mcpCapabilities.sse, false);
  assert.deepEqual(capabilities.sessionCapabilities, {
    list: true,
    close: true
  });
  assert.deepEqual(capabilities.auth, {
    logout: true
  });
});

test("treats malformed capability values as unsupported", () => {
  const capabilities = normalizeAgentCapabilities({
    loadSession: "yes",
    promptCapabilities: {
      image: "true",
      audio: 1,
      embeddedContext: null
    }
  });

  assert.equal(capabilities.loadSession, false);
  assert.deepEqual(capabilities.promptCapabilities, {
    image: false,
    audio: false,
    embeddedContext: false
  });
});
