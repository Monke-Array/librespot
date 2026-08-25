#!/usr/bin/env node
"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");

const { buildCapabilityDiffReport, summarizeMixerEndpointStatus } = require("../lib/capabilities");
const { evaluateWithPowerShell, listTargets } = require("../lib/cdp");
const {
  attachClassification,
  corpusSubdirs,
  ensureDir,
  loadCorpusRecords,
  readJson,
  saveOfficialCapture,
  timestampForFile,
  writeJson,
} = require("../lib/corpus");
const { summarizeCoverage } = require("../lib/report");
const { cargoBuildSpotifyd, runSpotifydFor } = require("../lib/spotifyd");
const { parseSpotifydLog } = require("../lib/spotifyd-log");

const DEFAULT_TEST_PAIR = {
  firstUri: "spotify:track:2BMRUAA1oTc7e9JPlr6xbZ",
  secondUri: "spotify:track:5g9lS8deSIxItFBmZRC4vN",
};

function repoRoot() {
  return path.resolve(__dirname, "..", "..", "..");
}

function defaultCorpusRoot() {
  return path.join(repoRoot(), "tools", "spotify-mixer-harness", "corpus");
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let index = 0; index < rest.length; index++) {
    const arg = rest[index];
    if (!arg.startsWith("--")) {
      (options._ ??= []).push(arg);
      continue;
    }
    const key = arg.slice(2).replace(/-([a-z])/g, (_, chr) => chr.toUpperCase());
    const next = rest[index + 1];
    if (!next || next.startsWith("--")) {
      options[key] = true;
    } else {
      options[key] = next;
      index++;
    }
  }
  return { command, options };
}

function printJson(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function requireString(options, key) {
  if (!options[key] || options[key] === true) throw new Error(`missing --${key.replace(/[A-Z]/g, (chr) => `-${chr.toLowerCase()}`)}`);
  return String(options[key]);
}

function optionalNumber(options, key) {
  if (options[key] === undefined || options[key] === true) return undefined;
  const value = Number(options[key]);
  if (!Number.isFinite(value)) throw new Error(`invalid --${key.replace(/[A-Z]/g, (chr) => `-${chr.toLowerCase()}`)}: ${options[key]}`);
  return value;
}

function controlCall(name, payload) {
  if (payload === undefined) return `window.spotifyMixerOracle.controls.${name}()`;
  return `window.spotifyMixerOracle.controls.${name}(${JSON.stringify(payload)})`;
}

function findSpotifyExe() {
  const appData = process.env.APPDATA;
  const candidates = [
    appData && path.join(appData, "Spotify", "Spotify.exe"),
    path.join(os.homedir(), "AppData", "Roaming", "Spotify", "Spotify.exe"),
  ].filter(Boolean);
  return candidates.find((candidate) => fs.existsSync(candidate)) ?? candidates[0];
}

function startSpotify({ port = 9222, extraArgs = [] } = {}) {
  const exe = findSpotifyExe();
  if (!fs.existsSync(exe)) throw new Error(`Spotify.exe was not found at ${exe}`);
  const args = [
    `--remote-debugging-port=${port}`,
    "--remote-allow-origins=*",
    "--disable-features=RendererCodeIntegrity",
    ...extraArgs,
  ];
  const child = spawn(exe, args, {
    detached: true,
    stdio: "ignore",
    windowsHide: true,
  });
  child.unref();
  return {
    exe,
    args,
    pid: child.pid,
  };
}

function installOracle(options) {
  const port = Number(options.port ?? 9222);
  const started = options.startSpotify ? startSpotify({ port }) : null;
  const expressionPath = path.join(repoRoot(), "tools", "spotify-mixer-harness", "xpui", "mixer-oracle.js");
  const installed = evaluateWithPowerShell({
    repoRoot: repoRoot(),
    port,
    expressionPath,
  });
  return { started, installed };
}

function xpuiEval(options, expression) {
  return evaluateWithPowerShell({
    repoRoot: repoRoot(),
    port: Number(options.port ?? 9222),
    expression,
  });
}

function snapshotOfficial(options) {
  const capture = xpuiEval(
    options,
    "window.spotifyMixerOracle ? window.spotifyMixerOracle.snapshot() : (() => { throw new Error('spotifyMixerOracle is not installed'); })()",
  );
  const saved = saveOfficialCapture({
    corpusRoot: String(options.corpus ?? defaultCorpusRoot()),
    capture,
    source: {
      client: "official Spotify Desktop XPUI via CDP",
      port: Number(options.port ?? 9222),
    },
  });
  return {
    file: saved.file,
    transitions: saved.capture.records.length,
    rawSummary: saved.capture.rawSummary,
  };
}

async function sampleOfficial(options) {
  const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
  const durationMs = Number(options.durationMs ?? 180000);
  const intervalMs = Number(options.intervalMs ?? 5000);
  const advanceSettleMs = Number(options.advanceSettleMs ?? 1500);
  const maxTransitions = Number(options.maxTransitions ?? 50);
  const staleLimit = Number(options.staleLimit ?? 10);
  if (options.install) installOracle(options);

  let initialControlStatus = null;
  const shouldAdvance = Boolean(options.advance || options.driveCurrentContext);
  if (options.driveCurrentContext) {
    initialControlStatus = xpuiEval(options, "window.spotifyMixerOracle.controls.status()");
    if (!initialControlStatus?.officialPlayer?.available) {
      throw new Error("Spotify Desktop official PlayerAPI is unavailable; install the oracle after XPUI finishes loading");
    }
    if (!initialControlStatus?.officialPlayer?.context?.isMixerEnabled) {
      throw new Error("Spotify Desktop is not in a Mixer-enabled context; play a Mixer-enabled playlist once, then rerun sample-official --drive-current-context");
    }
    xpuiEval(options, "window.spotifyMixerOracle.controls.mute()");
  }

  const startedAt = Date.now();
  const savedFiles = [];
  const seenSignatures = new Set();
  let transitions = 0;
  let consecutiveNoNew = 0;
  let snapshots = 0;
  const controlFailures = [];

  while (Date.now() - startedAt < durationMs && transitions < maxTransitions && consecutiveNoNew < staleLimit) {
    snapshots++;
    const capture = xpuiEval(
      options,
      "window.spotifyMixerOracle ? window.spotifyMixerOracle.snapshot() : (() => { throw new Error('spotifyMixerOracle is not installed'); })()",
    );
    const transitionCount = Array.isArray(capture.transitions)
      ? capture.transitions.length
      : Array.isArray(capture.records)
        ? capture.records.length
        : 0;
    if (transitionCount === 0 && !options.includeEmpty) {
      consecutiveNoNew++;
      if (Date.now() - startedAt + intervalMs < durationMs) await sleep(intervalMs);
      continue;
    }
    const saved = saveOfficialCapture({
      corpusRoot,
      capture,
      source: {
        client: "official Spotify Desktop XPUI via CDP",
        port: Number(options.port ?? 9222),
        sampler: true,
      },
    });
    const hashes = saved.capture.records.map((record) => record.signature.hash);
    const newHashes = hashes.filter((hash) => !seenSignatures.has(hash));
    hashes.forEach((hash) => seenSignatures.add(hash));
    transitions += saved.capture.records.length;
    savedFiles.push(saved.file);
    consecutiveNoNew = newHashes.length === 0 ? consecutiveNoNew + 1 : 0;
    if (shouldAdvance && Date.now() - startedAt + advanceSettleMs < durationMs) {
      try {
        xpuiEval(options, "window.spotifyMixerOracle.controls.next()");
        await sleep(advanceSettleMs);
      } catch (error) {
        controlFailures.push(String(error.message ?? error));
        break;
      }
    }
    if (Date.now() - startedAt + intervalMs < durationMs) await sleep(intervalMs);
  }

  const result = {
    format: "spotify-mixer-official-sampling-run-v1",
    generatedAt: new Date().toISOString(),
    corpusRoot,
    snapshots,
    transitions,
    distinctDspSignatures: seenSignatures.size,
    savedFiles,
    control: {
      driveCurrentContext: Boolean(options.driveCurrentContext),
      advancedBetweenSnapshots: shouldAdvance,
      initialStatus: initialControlStatus,
      failures: controlFailures,
    },
    stoppedBecause:
      controlFailures.length > 0
        ? "control-failure"
        : transitions >= maxTransitions
          ? "max-transitions"
          : consecutiveNoNew >= staleLimit
            ? "mostly-repeating"
            : "duration",
  };
  const out = options.out
    ? String(options.out)
    : path.join(corpusSubdirs(corpusRoot).reports, `official-sampling-${timestampForFile()}.json`);
  writeJson(out, result);
  return { out, ...result };
}

function buildControlExpression(options) {
  const action = requireString(options, "action");
  switch (action) {
    case "status":
      return controlCall("status");
    case "mute":
      return controlCall("mute");
    case "pause":
      return controlCall("pause");
    case "next":
      return controlCall("next");
    case "seek":
      return controlCall("seek", Number(options.positionMs ?? 0));
    case "play-uri":
      return controlCall("playUri", requireString(options, "uri"));
    case "play-context": {
      const payload = {
        contextUri: requireString(options, "contextUri"),
      };
      if (options.skipUri && options.skipUri !== true) payload.skipUri = String(options.skipUri);
      const skipIndex = optionalNumber(options, "skipIndex");
      if (skipIndex !== undefined) payload.skipIndex = skipIndex;
      const seekMs = optionalNumber(options, "seekMs");
      if (seekMs !== undefined) payload.seekMs = seekMs;
      if (options.paused) payload.paused = true;
      return controlCall("playContext", payload);
    }
    case "play-pair": {
      const payload = {
        firstUri: options.firstUri && options.firstUri !== true ? String(options.firstUri) : DEFAULT_TEST_PAIR.firstUri,
        secondUri: options.secondUri && options.secondUri !== true ? String(options.secondUri) : DEFAULT_TEST_PAIR.secondUri,
      };
      const seekMs = optionalNumber(options, "seekMs");
      if (seekMs !== undefined) payload.seekMs = seekMs;
      if (options.seekNearTransition) payload.seekNearTransition = true;
      if (options.paused) payload.paused = true;
      return controlCall("playPair", payload);
    }
    case "transfer": {
      if (!options.deviceName && !options.deviceId) throw new Error("missing --device-name or --device-id");
      const payload = {};
      if (options.deviceName && options.deviceName !== true) payload.deviceName = String(options.deviceName);
      if (options.deviceId && options.deviceId !== true) payload.deviceId = String(options.deviceId);
      return controlCall("transfer", payload);
    }
    default:
      throw new Error(`unknown official-control action: ${action}`);
  }
}

function controlActionForCommand(command) {
  return {
    "control-status": "status",
    "play-context": "play-context",
    "play-pair": "play-pair",
    next: "next",
    seek: "seek",
    transfer: "transfer",
  }[command];
}

function readLocalResult(options) {
  if (!options.spotifydRun) return null;
  const run = readJson(String(options.spotifydRun));
  if (run.parsed) return run.parsed;
  return parseSpotifydLog(`${run.stdout ?? ""}\n${run.stderr ?? ""}`, { exitCode: run.exitCode });
}

function classifyCorpus(options) {
  const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
  const records = loadCorpusRecords(corpusRoot);
  const localResult = readLocalResult(options);
  const classified = attachClassification(records, {
    localResult: localResult ?? {},
    devEqBypass: Boolean(options.devEqBypass),
    devFilterBypass: Boolean(options.devFilterBypass),
  });
  const report = summarizeCoverage(classified);
  const out = options.out
    ? String(options.out)
    : path.join(corpusSubdirs(corpusRoot).reports, `coverage-${timestampForFile()}.json`);
  writeJson(out, {
    format: "spotify-mixer-coverage-report-v1",
    generatedAt: new Date().toISOString(),
    corpusRoot,
    devEqBypass: Boolean(options.devEqBypass),
    devFilterBypass: Boolean(options.devFilterBypass),
    report,
    records: classified,
  });
  return { out, report };
}

function compare(options) {
  const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
  const localResult = readLocalResult(options) ?? {};
  const records = attachClassification(loadCorpusRecords(corpusRoot), {
    localResult,
    devEqBypass: Boolean(options.devEqBypass),
    devFilterBypass: Boolean(options.devFilterBypass),
  });
  const report = summarizeCoverage(records);
  const out = options.out
    ? String(options.out)
    : path.join(corpusSubdirs(corpusRoot).reports, `comparison-${timestampForFile()}.json`);
  writeJson(out, {
    format: "spotify-mixer-comparison-v1",
    generatedAt: new Date().toISOString(),
    corpusRoot,
    spotifydRun: options.spotifydRun ?? null,
    devEqBypass: Boolean(options.devEqBypass),
    devFilterBypass: Boolean(options.devFilterBypass),
    localResult,
    report,
    records,
  });
  return { out, report };
}

function capabilityDiff(options) {
  if (!options.spotifydRun) throw new Error("capability-diff requires --spotifyd-run PATH");
  const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
  const run = readJson(String(options.spotifydRun));
  const logText = `${run.stdout ?? ""}\n${run.stderr ?? ""}`;
  const report = buildCapabilityDiffReport({
    logText,
    officialSource: options.officialSource && options.officialSource !== true ? String(options.officialSource) : undefined,
    spotifydSource: options.spotifydSource && options.spotifydSource !== true ? String(options.spotifydSource) : undefined,
  });
  const out = options.out
    ? String(options.out)
    : path.join(corpusSubdirs(corpusRoot).reports, `capability-diff-${timestampForFile()}.json`);
  writeJson(out, report);
  return { out, differences: report.differences.length, report };
}

async function capabilityExperiment(options) {
  const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
  const dirs = corpusSubdirs(corpusRoot);
  const label = String(options.label ?? "capability-experiment");
  const capabilityOverrides =
    options.capabilityOverrides && options.capabilityOverrides !== true ? String(options.capabilityOverrides) : "";
  const env = {};
  if (capabilityOverrides) {
    env.LIBRESPOT_DEV_CONNECT_CAPABILITY_OVERRIDES = capabilityOverrides;
  }

  const beforeStatus = xpuiEval(options, controlCall("status"));
  const runPromise = runSpotifydFor({
    spotifydRoot: requireString(options, "spotifydRoot"),
    outDir: dirs.runs,
    durationMs: Number(options.durationMs ?? 30000),
    env,
    options: {
      deviceName: String(options.deviceName ?? "spotifyd-transition DEV"),
      deviceType:
        options.deviceType && options.deviceType !== true
          ? String(options.deviceType)
          : undefined,
      backend: String(options.backend ?? "rodio"),
      initialVolume: options.initialVolume === undefined ? 0 : Number(options.initialVolume),
      volumeController: options.volumeController === undefined ? "none" : String(options.volumeController),
      disableDiscovery: Boolean(options.disableDiscovery),
      noAudioCache: Boolean(options.noAudioCache),
    },
  });

  await sleep(Number(options.discoveryMs ?? 8000));
  let transferResult = null;
  let transferError = null;
  try {
    transferResult = xpuiEval(
      options,
      controlCall("transfer", {
        deviceName: String(options.deviceName ?? "spotifyd-transition DEV"),
      }),
    );
  } catch (error) {
    transferError = String(error.stack ?? error.message ?? error);
  }

  await sleep(Number(options.settleMs ?? 4000));
  let afterStatus = null;
  let afterError = null;
  try {
    afterStatus = xpuiEval(options, controlCall("status"));
  } catch (error) {
    afterError = String(error.stack ?? error.message ?? error);
  }

  const runResult = await runPromise;
  const result = {
    format: "spotify-mixer-capability-experiment-v1",
    generatedAt: new Date().toISOString(),
    label,
    capabilityOverrides,
    deviceName: String(options.deviceName ?? "spotifyd-transition DEV"),
    before: summarizeMixerEndpointStatus(beforeStatus),
    transfer: {
      ok: !transferError,
      result: transferResult,
      error: transferError,
    },
    after: afterStatus ? summarizeMixerEndpointStatus(afterStatus) : null,
    afterError,
    spotifydRun: runResult.file,
    spotifyd: {
      env: runResult.run.env,
      parsed: runResult.run.parsed,
      reason: runResult.run.reason,
      exitCode: runResult.run.exitCode,
      signal: runResult.run.signal,
    },
  };
  const out = options.out
    ? String(options.out)
    : path.join(dirs.reports, `capability-experiment-${timestampForFile()}.json`);
  writeJson(out, result);
  return {
    out,
    spotifydRun: runResult.file,
    label,
    capabilityOverrides,
    before: result.before,
    transfer: result.transfer,
    after: result.after,
    spotifyd: result.spotifyd,
  };
}

async function main() {
  const { command, options } = parseArgs(process.argv.slice(2));
  if (!command || command === "help" || command === "--help") {
    process.stdout.write(`Spotify Mixer harness

Commands:
  targets --port 9222
  start-spotify --port 9222
  install-oracle --port 9222 [--start-spotify]
  control-status --port 9222
  play-context --context-uri spotify:... [--skip-uri spotify:...] [--skip-index N] [--seek-ms N] [--paused]
  play-pair [--first-uri spotify:...] [--second-uri spotify:...] [--seek-near-transition]
  next --port 9222
  seek --position-ms N --port 9222
  transfer --device-name NAME|--device-id ID --port 9222
  official-control --action status|mute|pause|next|seek|play-uri|play-context|play-pair|transfer [options]
  snapshot-official --port 9222 [--corpus PATH]
  sample-official --port 9222 [--corpus PATH] [--duration-ms N] [--interval-ms N] [--drive-current-context] [--advance]
  classify-corpus [--corpus PATH] [--spotifyd-run PATH] [--dev-eq-bypass] [--dev-filter-bypass] [--out PATH]
  build-spotifyd --spotifyd-root PATH
  run-spotifyd --spotifyd-root PATH [--corpus PATH] [--duration-ms N] [--device-name NAME]
    [--capability-overrides "supports-dj=1,..."] [--device-type computer]
  compare [--corpus PATH] --spotifyd-run PATH [--dev-eq-bypass] [--dev-filter-bypass] [--out PATH]
  capability-diff --spotifyd-run PATH [--corpus PATH] [--out PATH]
  capability-experiment --spotifyd-root PATH [--label NAME] [--capability-overrides "supports-dj=1,..."] [--device-type computer]
`);
    return;
  }

  if (command === "targets") {
    printJson(await listTargets(Number(options.port ?? 9222)));
    return;
  }

  if (command === "start-spotify") {
    printJson(startSpotify({ port: Number(options.port ?? 9222) }));
    return;
  }

  if (command === "install-oracle") {
    printJson(installOracle(options));
    return;
  }

  if (command === "official-control") {
    printJson(xpuiEval(options, buildControlExpression(options)));
    return;
  }

  const controlAction = controlActionForCommand(command);
  if (controlAction) {
    printJson(xpuiEval(options, buildControlExpression({ ...options, action: controlAction })));
    return;
  }

  if (command === "snapshot-official") {
    printJson(snapshotOfficial(options));
    return;
  }

  if (command === "sample-official") {
    printJson(await sampleOfficial(options));
    return;
  }

  if (command === "classify-corpus") {
    printJson(classifyCorpus(options));
    return;
  }

  if (command === "build-spotifyd") {
    const result = cargoBuildSpotifyd({ spotifydRoot: requireString(options, "spotifydRoot") });
    printJson(result);
    process.exitCode = result.ok ? 0 : 1;
    return;
  }

  if (command === "run-spotifyd") {
    const corpusRoot = String(options.corpus ?? defaultCorpusRoot());
    const dirs = corpusSubdirs(corpusRoot);
    const env = {};
    if (options.capabilityOverrides && options.capabilityOverrides !== true) {
      env.LIBRESPOT_DEV_CONNECT_CAPABILITY_OVERRIDES = String(options.capabilityOverrides);
    }
    const result = await runSpotifydFor({
      spotifydRoot: requireString(options, "spotifydRoot"),
      outDir: dirs.runs,
      durationMs: Number(options.durationMs ?? 60000),
      env,
      options: {
        deviceName: String(options.deviceName ?? "spotifyd-transition DEV"),
        deviceType:
          options.deviceType && options.deviceType !== true
            ? String(options.deviceType)
            : undefined,
        backend: String(options.backend ?? "rodio"),
        initialVolume: options.initialVolume === undefined ? 0 : Number(options.initialVolume),
        volumeController: options.volumeController === undefined ? "none" : String(options.volumeController),
        disableDiscovery: Boolean(options.disableDiscovery),
        noAudioCache: Boolean(options.noAudioCache),
      },
    });
    printJson({ file: result.file, parsed: result.run.parsed, reason: result.run.reason });
    return;
  }

  if (command === "compare") {
    if (!options.spotifydRun) throw new Error("compare requires --spotifyd-run PATH");
    printJson(compare(options));
    return;
  }

  if (command === "capability-diff") {
    printJson(capabilityDiff(options));
    return;
  }

  if (command === "capability-experiment") {
    printJson(await capabilityExperiment(options));
    return;
  }

  throw new Error(`unknown command: ${command}`);
}

if (require.main === module) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}

module.exports = {
  DEFAULT_TEST_PAIR,
  buildControlExpression,
  capabilityDiff,
  capabilityExperiment,
};
