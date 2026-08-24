"use strict";

const SUPPORTED_STATUSES = new Set(["SUPPORTED", "DEV-EQ-BYPASS"]);

function increment(map, key, amount = 1) {
  const normalized = key ?? "UNKNOWN";
  map[normalized] = (map[normalized] ?? 0) + amount;
}

function round2(value) {
  return Math.round(value * 100) / 100;
}

function summarizeCoverage(records) {
  const signatures = new Set();
  const statusCounts = {};
  const presetCoverage = {};
  const blockerCounts = {};
  const effectCoverage = {};
  const transitionLengths = {};
  let supported = 0;

  for (const record of records) {
    if (record?.signature?.hash) signatures.add(record.signature.hash);
    const status = record?.classification?.status ?? "UNKNOWN";
    increment(statusCounts, status);
    if (SUPPORTED_STATUSES.has(status)) supported++;

    const summary = record?.signature?.summary ?? {};
    if (summary.presetId !== undefined && summary.presetId !== null) {
      increment(presetCoverage, String(summary.presetId));
    }
    for (const effect of summary.effects ?? []) increment(effectCoverage, effect);
    if (summary.transitionMs !== undefined && summary.transitionMs !== null) {
      increment(transitionLengths, String(summary.transitionMs));
    }
    for (const blocker of record?.classification?.blockers ?? []) {
      increment(blockerCounts, blocker);
    }
  }

  const largestBlockers = Object.entries(blockerCounts)
    .map(([blocker, count]) => ({ blocker, count }))
    .sort((left, right) => right.count - left.count || left.blocker.localeCompare(right.blocker));

  return {
    transitionsSampled: records.length,
    distinctDspSignatures: signatures.size,
    statusCounts,
    presetCoverage,
    effectCoverage,
    transitionLengths,
    largestBlockers,
    supportedCorpusPercentage: records.length === 0 ? 0 : round2((supported / records.length) * 100),
  };
}

module.exports = {
  summarizeCoverage,
};

