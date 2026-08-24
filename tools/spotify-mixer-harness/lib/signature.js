"use strict";

const crypto = require("crypto");

const { stable, stableStringify } = require("./stable-json");
const {
  MATERIALIZED_KEYS,
  eqBands,
  filterKeys,
  fxKeys,
  unknownAudioFields,
} = require("./materialized");

function parseMaybeJson(value) {
  if (typeof value !== "string") return value;
  const trimmed = value.trim();
  if (!trimmed || !"[{".includes(trimmed[0])) return value;
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
}

function numericString(value) {
  if (typeof value !== "string") return value;
  if (!/^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value.trim())) return value;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : value;
}

function normalizeFieldValue(value) {
  return stable(numericString(parseMaybeJson(value)));
}

function field(fields, key) {
  return fields?.[key];
}

function normalizedFields(fields, omittedKeys = []) {
  const omitted = new Set(omittedKeys);
  return stable(
    Object.fromEntries(
      Object.entries(fields ?? {})
        .filter(([key]) => !omitted.has(key))
        .map(([key, value]) => [key, normalizeFieldValue(value)]),
    ),
  );
}

function pointCount(value) {
  const parsed = parseMaybeJson(value);
  return Array.isArray(parsed) ? parsed.length : 0;
}

function inferEffects(materialized) {
  const outgoing = materialized?.outgoing?.fields ?? {};
  const incoming = materialized?.incoming?.fields ?? {};
  const effects = [];
  if (field(outgoing, MATERIALIZED_KEYS.FADE_OUT_CURVES) || field(incoming, MATERIALIZED_KEYS.FADE_IN_CURVES)) {
    effects.push("volume");
  }
  if (eqBands(materialized).length > 0) effects.push("eq");
  if (filterKeys(materialized).length > 0) effects.push("filter");
  if (fxKeys(materialized).length > 0) effects.push("fx");
  if (field(incoming, MATERIALIZED_KEYS.SPEED_AUTOMATION)) effects.push("speed");
  return effects.sort();
}

function buildDspSignature(transition) {
  const materialized = transition?.materialized ?? {};
  const outgoing = materialized.outgoing?.fields ?? {};
  const incoming = materialized.incoming?.fields ?? {};
  const presetId =
    field(outgoing, MATERIALIZED_KEYS.AUTO_PRESET_ID) ??
    field(incoming, MATERIALIZED_KEYS.AUTO_PRESET_ID) ??
    null;
  const transitionMs =
    numericString(field(incoming, MATERIALIZED_KEYS.FADE_OVERLAP)) ??
    numericString(field(outgoing, MATERIALIZED_KEYS.FADE_OUT_DURATION)) ??
    null;
  const unknown = unknownAudioFields(materialized);

  const canonical = {
    format: "spotify-mixer-dsp-signature-v1",
    presetId,
    mode: field(outgoing, MATERIALIZED_KEYS.AUTOMIX_MODE) ?? null,
    incomingAudioAutomixMode: field(incoming, MATERIALIZED_KEYS.AUDIO_AUTOMIX_MODE) ?? null,
    onlyAllowFadeOnAdvance: field(outgoing, MATERIALIZED_KEYS.ONLY_ALLOW_FADE_ON_ADVANCE) ?? null,
    timing: {
      fadeOutStartTime: numericString(field(outgoing, MATERIALIZED_KEYS.FADE_OUT_START) ?? null),
      fadeOutDuration: numericString(field(outgoing, MATERIALIZED_KEYS.FADE_OUT_DURATION) ?? null),
      fadeInStartTime: numericString(field(incoming, MATERIALIZED_KEYS.FADE_IN_START) ?? null),
      fadeInDuration: numericString(field(incoming, MATERIALIZED_KEYS.FADE_IN_DURATION) ?? null),
      fadeOverlap: numericString(field(incoming, MATERIALIZED_KEYS.FADE_OVERLAP) ?? null),
    },
    outgoingFields: normalizedFields(outgoing, [MATERIALIZED_KEYS.TRANSITION_URI]),
    incomingFields: normalizedFields(incoming, [MATERIALIZED_KEYS.TRANSITION_URI]),
  };

  const hash = crypto.createHash("sha256").update(stableStringify(canonical)).digest("hex");

  return {
    hash,
    canonical,
    summary: {
      presetId,
      transitionMs,
      effects: inferEffects(materialized),
      eqBands: eqBands(materialized),
      filterKeys: filterKeys(materialized),
      fxKeys: fxKeys(materialized),
      speedPointCount: pointCount(field(incoming, MATERIALIZED_KEYS.SPEED_AUTOMATION)),
      unknownAudioFields: unknown,
    },
  };
}

module.exports = {
  buildDspSignature,
  parseMaybeJson,
};

