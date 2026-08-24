"use strict";

const fs = require("fs");
const path = require("path");

const { sanitizeForCorpus } = require("./sanitize");
const { buildDspSignature } = require("./signature");
const { classifyTransition } = require("./classify");

function ensureDir(directory) {
  fs.mkdirSync(directory, { recursive: true });
}

function timestampForFile(date = new Date()) {
  return date.toISOString().replace(/[:.]/g, "-");
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, value) {
  ensureDir(path.dirname(file));
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function corpusSubdirs(corpusRoot) {
  return {
    root: corpusRoot,
    captures: path.join(corpusRoot, "captures"),
    reports: path.join(corpusRoot, "reports"),
    runs: path.join(corpusRoot, "runs"),
  };
}

function discoverCaptureFiles(corpusRoot) {
  const captures = corpusSubdirs(corpusRoot).captures;
  if (!fs.existsSync(captures)) return [];
  return fs
    .readdirSync(captures)
    .filter((name) => name.endsWith(".json"))
    .map((name) => path.join(captures, name))
    .sort();
}

function recordsFromCapture(capture) {
  if (Array.isArray(capture.records)) return capture.records;
  if (Array.isArray(capture.transitions)) return capture.transitions;
  if (capture.materialized) return [capture];
  return [];
}

function saveOfficialCapture({ corpusRoot, capture, source = {} }) {
  const dirs = corpusSubdirs(corpusRoot);
  ensureDir(dirs.captures);
  const sanitized = sanitizeForCorpus(capture);
  const records = recordsFromCapture(sanitized.value).map((record) => {
    const signature = record.signature ?? buildDspSignature(record);
    return {
      ...record,
      signature,
    };
  });
  const output = {
    format: "spotify-mixer-official-capture-v1",
    capturedAt: new Date().toISOString(),
    source,
    redactedPaths: sanitized.redactedPaths,
    records,
    rawSummary: sanitized.value.summary ?? null,
  };
  const file = path.join(dirs.captures, `official-${timestampForFile()}.json`);
  writeJson(file, output);
  return {
    file,
    capture: output,
  };
}

function loadCorpusRecords(corpusRoot) {
  const records = [];
  for (const file of discoverCaptureFiles(corpusRoot)) {
    const capture = readJson(file);
    for (const record of recordsFromCapture(capture)) {
      const signature = record.signature ?? buildDspSignature(record);
      records.push({
        ...record,
        corpusFile: file,
        signature,
      });
    }
  }
  return records;
}

function localResultForRecord(localResult, record) {
  if (!localResult?.events || !record?.pair) return localResult;
  const event = localResult.events.find((candidate) =>
    candidate.outgoingUri === record.pair.outgoingUri &&
    candidate.incomingUri === record.pair.incomingUri
  );
  if (!event) return localResult;
  return {
    ...localResult,
    selectedPath: event.selectedPath ?? localResult.selectedPath,
    matchedEvent: event,
  };
}

function attachClassification(records, options = {}) {
  return records.map((record) => ({
    ...record,
    classification: record.classification ?? classifyTransition({
      materialized: record.materialized,
      localResult: record.localResult ?? localResultForRecord(options.localResult ?? {}, record) ?? {},
      devEqBypass: options.devEqBypass ?? false,
    }),
  }));
}

module.exports = {
  attachClassification,
  corpusSubdirs,
  discoverCaptureFiles,
  ensureDir,
  loadCorpusRecords,
  localResultForRecord,
  readJson,
  saveOfficialCapture,
  timestampForFile,
  writeJson,
};
