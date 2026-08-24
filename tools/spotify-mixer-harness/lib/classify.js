"use strict";

const {
  hasEqAutomation,
  hasFilterAutomation,
  hasFxAutomation,
  unknownAudioFields,
} = require("./materialized");

const STATUSES = {
  SUPPORTED: "SUPPORTED",
  DEV_EQ_BYPASS: "DEV-EQ-BYPASS",
  UNSUPPORTED_EQ: "UNSUPPORTED-EQ",
  UNSUPPORTED_FILTER: "UNSUPPORTED-FILTER",
  UNSUPPORTED_FX: "UNSUPPORTED-FX",
  UNSUPPORTED_TIMING: "UNSUPPORTED-TIMING",
  UNSUPPORTED_SPEED: "UNSUPPORTED-SPEED",
  UNKNOWN: "UNKNOWN",
  RUNTIME_FAILURE: "RUNTIME-FAILURE",
};

function rejectionText(localResult) {
  return (localResult?.rejectionReasons ?? []).join("\n").toLowerCase();
}

function classifyTransition({ materialized, localResult = {}, devEqBypass = false }) {
  const blockers = [];
  const text = rejectionText(localResult);

  if ((localResult.runtimeFailures ?? []).length > 0) {
    return {
      status: STATUSES.RUNTIME_FAILURE,
      blockers: [...localResult.runtimeFailures],
    };
  }

  if (/timing|source durations|wall-clock|overlap|duration/.test(text)) {
    return {
      status: STATUSES.UNSUPPORTED_TIMING,
      blockers: ["timing"],
    };
  }

  if (/speed automation|unsupported tempo|speeda|speedb/.test(text)) {
    return {
      status: STATUSES.UNSUPPORTED_SPEED,
      blockers: ["speed"],
    };
  }

  if (hasFilterAutomation(materialized)) {
    blockers.push("filter");
    return {
      status: STATUSES.UNSUPPORTED_FILTER,
      blockers,
    };
  }

  if (hasFxAutomation(materialized) || unknownAudioFields(materialized).length > 0) {
    blockers.push("fx");
    return {
      status: STATUSES.UNSUPPORTED_FX,
      blockers,
    };
  }

  if (hasEqAutomation(materialized)) {
    if (devEqBypass && localResult.selectedPath === "materialized") {
      return {
        status: STATUSES.DEV_EQ_BYPASS,
        blockers,
      };
    }
    return {
      status: STATUSES.UNSUPPORTED_EQ,
      blockers: ["eq"],
    };
  }

  if (localResult.selectedPath === "materialized" || localResult.selectedPath === "saved") {
    return {
      status: STATUSES.SUPPORTED,
      blockers,
    };
  }

  if (text) {
    return {
      status: STATUSES.UNKNOWN,
      blockers: ["unknown"],
    };
  }

  return {
    status: STATUSES.UNKNOWN,
    blockers: ["not-run"],
  };
}

module.exports = {
  STATUSES,
  classifyTransition,
};

