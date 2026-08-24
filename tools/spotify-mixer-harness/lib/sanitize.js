"use strict";

const SENSITIVE_KEY = /authorization|cookie|credential|oauth|password|secret|session|token/i;
const BEARER_VALUE = /\bBearer\s+[-._~+/=A-Za-z0-9]+/g;

function isSensitiveKey(key) {
  return SENSITIVE_KEY.test(key);
}

function joinPath(parent, key) {
  return parent ? `${parent}.${key}` : String(key);
}

function sanitizeNode(value, path, redactedPaths) {
  if (Array.isArray(value)) {
    return value.map((entry, index) => sanitizeNode(entry, joinPath(path, index), redactedPaths));
  }

  if (value && typeof value === "object") {
    const output = {};
    for (const [key, entry] of Object.entries(value)) {
      const childPath = joinPath(path, key);
      if (isSensitiveKey(key)) {
        output[key] = "[REDACTED]";
        redactedPaths.push(childPath);
      } else {
        output[key] = sanitizeNode(entry, childPath, redactedPaths);
      }
    }
    return output;
  }

  if (typeof value === "string") {
    const sanitized = value.replace(BEARER_VALUE, "Bearer [REDACTED]");
    if (sanitized !== value) redactedPaths.push(path);
    return sanitized;
  }

  return value;
}

function sanitizeForCorpus(value) {
  const redactedPaths = [];
  return {
    value: sanitizeNode(value, "", redactedPaths),
    redactedPaths,
  };
}

function sanitizeLogText(text) {
  return String(text ?? "")
    .replace(/\bBearer\s+[-._~+/=A-Za-z0-9]+/g, "Bearer [REDACTED]")
    .replace(/Restoring previous login as user [^\r\n]+/g, "Restoring previous login as user [REDACTED_USER]")
    .replace(/Authenticated as '[^']*'/g, "Authenticated as '[REDACTED_USER]'")
    .replace(/Using device id '[^']*'/g, "Using device id '[REDACTED_DEVICE_ID]'")
    .replace(/\/devices\/[0-9a-f]{40}/gi, "/devices/[REDACTED_DEVICE_ID]")
    .replace(/\bconnect state for [0-9a-f]{40}/gi, "connect state for [REDACTED_DEVICE_ID]")
    .replace(/\bsession <[0-9a-f]{32}>/gi, "session <[REDACTED_SESSION_ID]>")
    .replace(/\bsalt=\d+/g, "salt=[REDACTED_SALT]")
    .replace(/\bconnection-id\s+[-._~+/=A-Za-z0-9]+/gi, "connection-id [REDACTED_CONNECTION_ID]")
    .replace(
      /\b(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[0-1])(?:\.\d{1,3}){2})(?=[:\s])/g,
      "[REDACTED_LOCAL_IP]",
    );
}

module.exports = {
  sanitizeForCorpus,
  sanitizeLogText,
};
