const REDACTED = "[REDACTED]";
const SENSITIVE_KEY_PATTERN = /(?:^|[_-])(api[_-]?key|auth|authorization|bearer|cookie|credential|password|private[_-]?key|secret|session[_-]?token|token)(?:$|[_-])/i;
const SENSITIVE_FLAG_PATTERN = /^--?(?:api[-_]?key|auth|authorization|cookie|credential|password|private[-_]?key|secret|session[-_]?token|token)$/i;

export function redactValue(value: unknown, seen = new WeakSet<object>()): unknown {
  if (typeof value === "string") {
    return redactString(value);
  }
  if (Array.isArray(value)) {
    return redactCommandArgs(value.map((item) => (typeof item === "string" ? item : redactValue(item, seen))));
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  if (seen.has(value)) {
    return "[Circular]";
  }
  seen.add(value);

  const entries = Object.entries(value as Record<string, unknown>).map(([key, nested]) => [
    key,
    isSensitiveKey(key) ? REDACTED : redactValue(nested, seen)
  ]);
  return Object.fromEntries(entries);
}

export function redactString(value: string): string {
  return value
    .replace(/(authorization\s*[:=]\s*bearer\s+)[^\s"'`]+/gi, `$1${REDACTED}`)
    .replace(/((?:api[_-]?key|auth|cookie|credential|password|private[_-]?key|secret|session[_-]?token|token)\s*[:=]\s*)[^\s"'`,}]+/gi, `$1${REDACTED}`)
    .replace(/(--?(?:api[-_]?key|auth|authorization|cookie|credential|password|private[-_]?key|secret|session[-_]?token|token)=)[^\s"'`]+/gi, `$1${REDACTED}`)
    .replace(/(--?(?:api[-_]?key|auth|authorization|cookie|credential|password|private[-_]?key|secret|session[-_]?token|token)\s+)[^\s"'`]+/gi, `$1${REDACTED}`);
}

export function redactCommandArgs(args: unknown[]): unknown[] {
  const redacted: unknown[] = [];
  let redactNext = false;
  for (const arg of args) {
    if (typeof arg !== "string") {
      redacted.push(arg);
      redactNext = false;
      continue;
    }
    if (redactNext) {
      redacted.push(REDACTED);
      redactNext = false;
      continue;
    }
    const [flag] = arg.split("=", 1);
    if (SENSITIVE_FLAG_PATTERN.test(flag || "")) {
      if (arg.includes("=")) {
        redacted.push(`${flag}=${REDACTED}`);
      } else {
        redacted.push(arg);
        redactNext = true;
      }
      continue;
    }
    redacted.push(redactString(arg));
  }
  return redacted;
}

export function redactedJson(value: unknown): string {
  return JSON.stringify(redactValue(value), null, 2);
}

function isSensitiveKey(key: string): boolean {
  return SENSITIVE_KEY_PATTERN.test(key);
}
