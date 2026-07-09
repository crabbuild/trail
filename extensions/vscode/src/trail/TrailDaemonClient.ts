import * as fs from "node:fs/promises";
import * as http from "node:http";
import * as https from "node:https";
import * as path from "node:path";
import { URL } from "node:url";

export interface DaemonEndpoint {
  url: string;
  authEnabled: boolean;
  token?: string | undefined;
}

export interface DaemonRequestOptions {
  timeoutMs?: number | undefined;
}

export class TrailDaemonClient {
  constructor(private readonly workspaceRoot: string) {}

  async discover(): Promise<DaemonEndpoint | undefined> {
    const endpointPath = path.join(this.workspaceRoot, ".trail", "daemon.json");
    try {
      const raw = await fs.readFile(endpointPath, "utf8");
      const parsed = JSON.parse(raw) as { url?: string; auth_enabled?: boolean; authEnabled?: boolean };
      if (!parsed.url) {
        return undefined;
      }
      const authEnabled = Boolean(parsed.auth_enabled ?? parsed.authEnabled);
      const token = authEnabled ? await this.readToken() : undefined;
      return { url: parsed.url, authEnabled, token };
    } catch {
      return undefined;
    }
  }

  async getJson<T>(endpoint: DaemonEndpoint, route: string, options: DaemonRequestOptions = {}): Promise<T> {
    return this.requestJson<T>(endpoint, "GET", route, undefined, options);
  }

  async postJson<T>(
    endpoint: DaemonEndpoint,
    route: string,
    body?: unknown,
    options: DaemonRequestOptions = {}
  ): Promise<T> {
    return this.requestJson<T>(endpoint, "POST", route, body, options);
  }

  async deleteJson<T>(endpoint: DaemonEndpoint, route: string, options: DaemonRequestOptions = {}): Promise<T> {
    return this.requestJson<T>(endpoint, "DELETE", route, undefined, options);
  }

  private async requestJson<T>(
    endpoint: DaemonEndpoint,
    method: "GET" | "POST" | "DELETE",
    route: string,
    body?: unknown,
    options: DaemonRequestOptions = {}
  ): Promise<T> {
    const url = new URL(route, endpoint.url);
    const payload = body === undefined ? undefined : JSON.stringify(body);
    const transport = url.protocol === "https:" ? https : http;
    const timeoutMs = options.timeoutMs ?? 10000;
    return new Promise<T>((resolve, reject) => {
      const request = transport.request(
        url,
        {
          method,
          headers: {
            ...(payload ? { "content-type": "application/json", "content-length": Buffer.byteLength(payload).toString() } : {}),
            ...(endpoint.token ? { authorization: `Bearer ${endpoint.token}` } : {})
          },
          timeout: timeoutMs
        },
        (response) => {
          let data = "";
          response.setEncoding("utf8");
          response.on("data", (chunk) => {
            data += chunk;
          });
          response.on("end", () => {
            if (!response.statusCode || response.statusCode >= 400) {
              reject(new Error(`Trail daemon ${method} ${route} failed: ${response.statusCode} ${data}`));
              return;
            }
            try {
              resolve(JSON.parse(data) as T);
            } catch (error) {
              reject(new Error(`Trail daemon returned invalid JSON for ${route}: ${String(error)}`));
            }
          });
        }
      );
      request.on("error", reject);
      request.on("timeout", () => {
        request.destroy(new Error(`Trail daemon ${method} ${route} timed out`));
      });
      if (payload) {
        request.write(payload);
      }
      request.end();
    });
  }

  private async readToken(): Promise<string | undefined> {
    try {
      return (await fs.readFile(path.join(this.workspaceRoot, ".trail", "daemon.token"), "utf8")).trim();
    } catch {
      return undefined;
    }
  }
}
