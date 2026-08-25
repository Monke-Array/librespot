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
  DEV_FILTER_BYPASS: "DEV-FILTER-BYPASS",
  DEV_EQ_FILTER_BYPASS: "DEV-EQ-FILTER-BYPASS",
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

function classifyTransition({ materialized, localResult = {}, devEqBypass = false, devFilterBypass = false }) {
  const blockers = [];
  const text = rejectionText(localResult);
  const eqAutomation = hasEqAutomation(materialized);
  const filterAutomation = hasFilterAutomation(materialized);
  const fxAutomation = hasFxAutomation(materialized) || unknownAudioFields(materialized).length > 0;

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

  if (filterAutomation && !devFilterBypass) {
    blockers.push("filter");
    return {
      status: STATUSES.UNSUPPORTED_FILTER,
      blockers,
    };
  }

  if (fxAutomation) {
    blockers.push("fx");
    return {
      status: STATUSES.UNSUPPORTED_FX,
      blockers,
    };
  }

  if (eqAutomation && !devEqBypass) {
    return {
      status: STATUSES.UNSUPPORTED_EQ,
      blockers: ["eq"],
    };
  }

  if (eqAutomation || filterAutomation) {
    if (devEqBypass && devFilterBypass && eqAutomation && filterAutomation) {
      return {
        status: STATUSES.DEV_EQ_FILTER_BYPASS,
        blockers,
      };
    }
    if (devFilterBypass && filterAutomation) {
      return {
        status: STATUSES.DEV_FILTER_BYPASS,
        blockers,
      };
    }
    if (devEqBypass && eqAutomation) {
      return {
        status: STATUSES.DEV_EQ_BYPASS,
        blockers,
      };
    }
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
