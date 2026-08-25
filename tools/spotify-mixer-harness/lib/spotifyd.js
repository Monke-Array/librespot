"use strict";

const fs = require("fs");
const path = require("path");
const { spawn, spawnSync } = require("child_process");

const { ensureDir, timestampForFile, writeJson } = require("./corpus");
const { sanitizeLogText } = require("./sanitize");
const { parseSpotifydLog } = require("./spotifyd-log");

const MATERIALIZED_BYPASS_ENV = {
  LIBRESPOT_DEV_BYPASS_MATERIALIZED_EQ: "1",
  LIBRESPOT_DEV_BYPASS_MATERIALIZED_FILTER: "1",
  LIBRESPOT_DEV_DUMP_MATERIALIZED_METADATA: "1",
};

function cargoBuildSpotifyd({ spotifydRoot, features = "rodio_backend" }) {
  const result = spawnSync("cargo", ["build", "--no-default-features", "--features", features], {
    cwd: spotifydRoot,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  return {
    ok: result.status === 0,
    status: result.status,
    signal: result.signal,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

function spotifydExecutable(spotifydRoot) {
  const exe = process.platform === "win32" ? "spotifyd.exe" : "spotifyd";
  return path.join(spotifydRoot, "target", "debug", exe);
}

function buildSpotifydArgs(options = {}) {
  const args = [
    "--no-daemon",
    "--verbose",
    "--backend",
    options.backend ?? "rodio",
    "--device-name",
    options.deviceName ?? "spotifyd-transition DEV",
  ];
  if (options.configPath) args.push("--config-path", options.configPath);
  if (options.cachePath) args.push("--cache-path", options.cachePath);
  if (options.initialVolume !== undefined) args.push("--initial-volume", String(options.initialVolume));
  if (options.volumeController) args.push("--volume-controller", options.volumeController);
  if (options.deviceType) args.push("--device-type", options.deviceType);
  if (options.disableDiscovery) args.push("--disable-discovery");
  if (options.noAudioCache) args.push("--no-audio-cache");
  return args;
}

function runSpotifydFor({ spotifydRoot, outDir, durationMs = 60000, env = {}, args = [], options = {} }) {
  const exe = spotifydExecutable(spotifydRoot);
  if (!fs.existsSync(exe)) {
    throw new Error(`spotifyd executable not found: ${exe}. Run build-spotifyd first.`);
  }
  ensureDir(outDir);
  const startedAt = new Date();
  const devEnv = Object.fromEntries(
    Object.entries({
      ...MATERIALIZED_BYPASS_ENV,
      ...env,
    }).filter(([key]) => key.startsWith("LIBRESPOT_DEV_")),
  );
  const child = spawn(exe, args.length ? args : buildSpotifydArgs(options), {
    cwd: spotifydRoot,
    env: {
      ...process.env,
      ...devEnv,
    },
    windowsHide: true,
  });

  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString();
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });

  return new Promise((resolve) => {
    let settled = false;
    const finish = (reason, code, signal) => {
      if (settled) return;
      settled = true;
      const sanitizedStdout = sanitizeLogText(stdout);
      const sanitizedStderr = sanitizeLogText(stderr);
      const logText = `${sanitizedStdout}\n${sanitizedStderr}`;
      const parsed = parseSpotifydLog(logText, { exitCode: code });
      const run = {
        format: "spotify-mixer-spotifyd-run-v1",
        startedAt: startedAt.toISOString(),
        endedAt: new Date().toISOString(),
        reason,
        exitCode: code,
        signal,
        args: args.length ? args : buildSpotifydArgs(options),
        env: devEnv,
        parsed,
        stdout: sanitizedStdout,
        stderr: sanitizedStderr,
      };
      const file = path.join(outDir, `spotifyd-${timestampForFile(startedAt)}.json`);
      writeJson(file, run);
      resolve({ file, run });
    };

    child.on("exit", (code, signal) => finish("exit", code, signal));
    setTimeout(() => {
      if (!child.killed) child.kill();
      finish("timeout", null, null);
    }, durationMs).unref();
  });
}

module.exports = {
  buildSpotifydArgs,
  cargoBuildSpotifyd,
  runSpotifydFor,
  spotifydExecutable,
};
