import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { TrailDaemonClient } from "../trail/TrailDaemonClient";

test("TrailDaemonClient discovers endpoint and sends bearer token", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "trail-daemon-client-"));
  const crabDir = path.join(tempDir, ".trail");
  fs.mkdirSync(crabDir);
  fs.writeFileSync(path.join(crabDir, "daemon.token"), "secret-token\n");

  let authHeader = "";
  const server = http.createServer((request, response) => {
    authHeader = String(request.headers.authorization || "");
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ ok: true, route: request.url }));
  });
  await listen(server);

  try {
    const address = server.address();
    assert.ok(address && typeof address === "object");
    fs.writeFileSync(
      path.join(crabDir, "daemon.json"),
      JSON.stringify({
        url: `http://127.0.0.1:${address.port}`,
        auth_enabled: true
      })
    );

    const client = new TrailDaemonClient(tempDir);
    const endpoint = await client.discover();
    assert.equal(endpoint?.authEnabled, true);
    assert.equal(endpoint?.token, "secret-token");
    assert.equal((await client.getJson<{ route: string }>(endpoint!, "/v1/health")).route, "/v1/health");
    assert.equal(authHeader, "Bearer secret-token");
  } finally {
    await close(server);
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("TrailDaemonClient posts JSON bodies", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "trail-daemon-client-"));
  const crabDir = path.join(tempDir, ".trail");
  fs.mkdirSync(crabDir);

  let body = "";
  const server = http.createServer((request, response) => {
    request.setEncoding("utf8");
    request.on("data", (chunk) => {
      body += chunk;
    });
    request.on("end", () => {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ received: JSON.parse(body) }));
    });
  });
  await listen(server);

  try {
    const address = server.address();
    assert.ok(address && typeof address === "object");
    fs.writeFileSync(
      path.join(crabDir, "daemon.json"),
      JSON.stringify({
        url: `http://127.0.0.1:${address.port}`,
        auth_enabled: false
      })
    );

    const client = new TrailDaemonClient(tempDir);
    const endpoint = await client.discover();
    const result = await client.postJson<{ received: unknown }>(endpoint!, "/v1/merge-queue", {
      source: "lane-a",
      target: "main"
    });
    assert.deepEqual(result.received, {
      source: "lane-a",
      target: "main"
    });
  } finally {
    await close(server);
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("TrailDaemonClient sends DELETE requests", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "trail-daemon-client-"));
  const crabDir = path.join(tempDir, ".trail");
  fs.mkdirSync(crabDir);

  let method = "";
  let route = "";
  const server = http.createServer((request, response) => {
    method = String(request.method || "");
    route = String(request.url || "");
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ ok: true }));
  });
  await listen(server);

  try {
    const address = server.address();
    assert.ok(address && typeof address === "object");
    fs.writeFileSync(
      path.join(crabDir, "daemon.json"),
      JSON.stringify({
        url: `http://127.0.0.1:${address.port}`,
        auth_enabled: false
      })
    );

    const client = new TrailDaemonClient(tempDir);
    const endpoint = await client.discover();
    assert.deepEqual(await client.deleteJson(endpoint!, "/v1/lanes/demo?force=true"), { ok: true });
    assert.equal(method, "DELETE");
    assert.equal(route, "/v1/lanes/demo?force=true");
  } finally {
    await close(server);
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

function listen(server: http.Server): Promise<void> {
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", resolve);
  });
}

function close(server: http.Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error);
        return;
      }
      resolve();
    });
  });
}
