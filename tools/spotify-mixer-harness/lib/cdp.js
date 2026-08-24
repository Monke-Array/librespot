"use strict";

const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

function httpJson(url, timeoutMs = 5000) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout: timeoutMs }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        if (response.statusCode < 200 || response.statusCode >= 300) {
          reject(new Error(`${url} returned HTTP ${response.statusCode}`));
          return;
        }
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(new Error(`invalid JSON from ${url}: ${error.message}`));
        }
      });
    });
    request.on("timeout", () => request.destroy(new Error(`timeout reading ${url}`)));
    request.on("error", reject);
  });
}

async function listTargets(port = 9222) {
  return httpJson(`http://127.0.0.1:${port}/json/list`);
}

function defaultCdpEvalPath(repoRoot) {
  return path.join(repoRoot, "tools", "spotify-automix-oracle", "cdp-eval.ps1");
}

function powershellExecutable() {
  return process.env.PWSH || "powershell";
}

function evaluateWithPowerShell({ repoRoot, port = 9222, expression, expressionPath }) {
  const cdpEval = defaultCdpEvalPath(repoRoot);
  let tempPath = null;
  const args = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", cdpEval, "-Port", String(port)];

  if (expressionPath) {
    args.push("-ExpressionPath", expressionPath);
  } else {
    tempPath = path.join(os.tmpdir(), `spotify-mixer-cdp-${process.pid}-${Date.now()}.js`);
    fs.writeFileSync(tempPath, expression);
    args.push("-ExpressionPath", tempPath);
  }

  try {
    const result = spawnSync(powershellExecutable(), args, {
      cwd: repoRoot,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error((result.stderr || result.stdout || `cdp-eval exited ${result.status}`).trim());
    }
    const stdout = result.stdout.trim();
    return stdout ? JSON.parse(stdout) : null;
  } finally {
    if (tempPath) {
      try {
        fs.unlinkSync(tempPath);
      } catch {
        // Temporary-file cleanup is best effort.
      }
    }
  }
}

module.exports = {
  evaluateWithPowerShell,
  listTargets,
};

