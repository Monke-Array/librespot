"use strict";

const SIDE_LABEL = {
  outgoing: "out",
  incoming: "in",
};

const MATERIALIZED_KEYS = {
  FADE_OUT_START: "audio.fade_out_start_time",
  FADE_OUT_DURATION: "audio.fade_out_duration",
  FADE_OUT_CURVES: "audio.fade_out_curves",
  FADE_OUT_EQ_LOW_GAIN_CURVES: "audio.fade_out_eq_low_gain_curves",
  FADE_OUT_EQ_MID_GAIN_CURVES: "audio.fade_out_eq_mid_gain_curves",
  FADE_OUT_EQ_HIGH_GAIN_CURVES: "audio.fade_out_eq_high_gain_curves",
  FADE_OUT_FILTER_CUTOFF_CURVES: "audio.fade_out_filter_cutoff_curves",
  FADE_OUT_FILTER_RESONANCE_CURVES: "audio.fade_out_filter_resonance_curves",
  FADE_IN_START: "audio.fade_in_start_time",
  FADE_IN_DURATION: "audio.fade_in_duration",
  FADE_IN_CURVES: "audio.fade_in_curves",
  FADE_IN_EQ_LOW_GAIN_CURVES: "audio.fade_in_eq_low_gain_curves",
  FADE_IN_EQ_MID_GAIN_CURVES: "audio.fade_in_eq_mid_gain_curves",
  FADE_IN_EQ_HIGH_GAIN_CURVES: "audio.fade_in_eq_high_gain_curves",
  FADE_IN_FILTER_CUTOFF_CURVES: "audio.fade_in_filter_cutoff_curves",
  FADE_IN_FILTER_RESONANCE_CURVES: "audio.fade_in_filter_resonance_curves",
  FADE_OVERLAP: "audio.fade_overlap",
  SPEED_AUTOMATION: "audio.speed_automation",
  ONLY_ALLOW_FADE_ON_ADVANCE: "audio.only_allow_fade_on_advance",
  AUDIO_AUTOMIX_MODE: "audio.automix_mode",
  AUTO_PRESET_ID: "automix.auto_preset_id",
  AUTOMIX_MODE: "automix.mode",
  TRANSITION_URI: "automix.transition_uri",
};

const KNOWN_MATERIALIZED_AUDIO_KEYS = new Set([
  MATERIALIZED_KEYS.FADE_OUT_START,
  MATERIALIZED_KEYS.FADE_OUT_DURATION,
  MATERIALIZED_KEYS.FADE_OUT_CURVES,
  MATERIALIZED_KEYS.FADE_OUT_EQ_LOW_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_OUT_EQ_MID_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_OUT_EQ_HIGH_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_OUT_FILTER_CUTOFF_CURVES,
  MATERIALIZED_KEYS.FADE_OUT_FILTER_RESONANCE_CURVES,
  MATERIALIZED_KEYS.FADE_IN_START,
  MATERIALIZED_KEYS.FADE_IN_DURATION,
  MATERIALIZED_KEYS.FADE_IN_CURVES,
  MATERIALIZED_KEYS.FADE_IN_EQ_LOW_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_IN_EQ_MID_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_IN_EQ_HIGH_GAIN_CURVES,
  MATERIALIZED_KEYS.FADE_IN_FILTER_CUTOFF_CURVES,
  MATERIALIZED_KEYS.FADE_IN_FILTER_RESONANCE_CURVES,
  MATERIALIZED_KEYS.FADE_OVERLAP,
  MATERIALIZED_KEYS.SPEED_AUTOMATION,
  MATERIALIZED_KEYS.ONLY_ALLOW_FADE_ON_ADVANCE,
  MATERIALIZED_KEYS.AUDIO_AUTOMIX_MODE,
]);

const MATERIALIZED_METADATA_KEYS = new Set([
  ...KNOWN_MATERIALIZED_AUDIO_KEYS,
  MATERIALIZED_KEYS.AUTO_PRESET_ID,
  MATERIALIZED_KEYS.AUTOMIX_MODE,
  MATERIALIZED_KEYS.TRANSITION_URI,
]);

const EQ_BANDS = [
  ["audio.fade_out_eq_low_gain_curves", "out.low"],
  ["audio.fade_out_eq_mid_gain_curves", "out.mid"],
  ["audio.fade_out_eq_high_gain_curves", "out.high"],
  ["audio.fade_in_eq_low_gain_curves", "in.low"],
  ["audio.fade_in_eq_mid_gain_curves", "in.mid"],
  ["audio.fade_in_eq_high_gain_curves", "in.high"],
];

const FILTER_KEYS = [
  "audio.fade_out_filter_cutoff_curves",
  "audio.fade_out_filter_resonance_curves",
  "audio.fade_in_filter_cutoff_curves",
  "audio.fade_in_filter_resonance_curves",
];

const FX_PATTERN = /^audio\.fade_(?:out|in)_(?!(?:curves|eq_(?:low|mid|high)_gain_curves|filter_(?:cutoff|resonance)_curves)$).+_curves$/;

function sideFields(materialized, side) {
  return materialized?.[side]?.fields ?? {};
}

function eachMaterializedField(materialized) {
  const fields = [];
  for (const side of ["outgoing", "incoming"]) {
    for (const [key, value] of Object.entries(sideFields(materialized, side))) {
      fields.push({
        side,
        sideLabel: SIDE_LABEL[side],
        key,
        value,
      });
    }
  }
  return fields;
}

function hasAnyKey(materialized, keys) {
  const keySet = new Set(keys);
  return eachMaterializedField(materialized).some((field) => keySet.has(field.key));
}

function eqBands(materialized) {
  const result = [];
  for (const [key, label] of EQ_BANDS) {
    if (hasAnyKey(materialized, [key])) result.push(label);
  }
  return result.sort();
}

function filterKeys(materialized) {
  return eachMaterializedField(materialized)
    .filter((field) => FILTER_KEYS.includes(field.key))
    .map((field) => field.key)
    .sort();
}

function unknownAudioFields(materialized) {
  return eachMaterializedField(materialized)
    .filter((field) => field.key.startsWith("audio.") && !KNOWN_MATERIALIZED_AUDIO_KEYS.has(field.key))
    .map((field) => field.key)
    .sort();
}

function fxKeys(materialized) {
  return eachMaterializedField(materialized)
    .filter((field) => FX_PATTERN.test(field.key))
    .map((field) => field.key)
    .sort();
}

function hasEqAutomation(materialized) {
  return eqBands(materialized).length > 0;
}

function hasFilterAutomation(materialized) {
  return filterKeys(materialized).length > 0;
}

function hasFxAutomation(materialized) {
  return fxKeys(materialized).length > 0;
}

function hasMaterializedEdge(fields) {
  return Object.keys(fields ?? {}).some((key) => MATERIALIZED_METADATA_KEYS.has(key));
}

function transitionFromTrackPair(outgoing, incoming) {
  const outgoingFields = outgoing?.metadata ?? outgoing?.fields ?? {};
  const incomingFields = incoming?.metadata ?? incoming?.fields ?? {};
  if (!hasMaterializedEdge(outgoingFields) && !hasMaterializedEdge(incomingFields)) return null;
  return {
    pair: {
      outgoingUri: outgoing?.uri ?? outgoingFields.uri ?? null,
      incomingUri: incoming?.uri ?? incomingFields.uri ?? null,
    },
    materialized: {
      outgoing: {
        uri: outgoing?.uri ?? outgoingFields.uri ?? null,
        fields: Object.fromEntries(
          Object.entries(outgoingFields).filter(([key]) => MATERIALIZED_METADATA_KEYS.has(key) || key.startsWith("audio.")),
        ),
      },
      incoming: {
        uri: incoming?.uri ?? incomingFields.uri ?? null,
        fields: Object.fromEntries(
          Object.entries(incomingFields).filter(([key]) => MATERIALIZED_METADATA_KEYS.has(key) || key.startsWith("audio.")),
        ),
      },
    },
  };
}

function extractTracksFromObject(root, options = {}) {
  const maxDepth = options.maxDepth ?? 10;
  const maxNodes = options.maxNodes ?? 15000;
  const tracks = [];
  const seen = new WeakSet();
  let visited = 0;

  function candidateFromObject(value, path) {
    const metadata = value?.metadata ?? value?.formatListAttributes ?? value?.format_list_attributes;
    if (!metadata || typeof metadata !== "object") return;
    if (!Object.keys(metadata).some((key) => key.startsWith("audio.") || key.startsWith("automix."))) return;
    tracks.push({
      path,
      uri: value.uri ?? value.trackUri ?? value.track_uri ?? metadata.uri ?? null,
      uid: value.uid ?? value.rowId ?? value.row_id ?? null,
      metadata,
    });
  }

  function walk(value, path, depth) {
    if (visited++ > maxNodes || depth > maxDepth) return;
    if (!value || typeof value !== "object") return;
    if (seen.has(value)) return;
    seen.add(value);
    candidateFromObject(value, path);
    if (Array.isArray(value)) {
      for (let index = 0; index < value.length; index++) {
        walk(value[index], `${path}[${index}]`, depth + 1);
      }
      return;
    }
    for (const [key, child] of Object.entries(value)) {
      if (typeof child === "function") continue;
      walk(child, path ? `${path}.${key}` : key, depth + 1);
    }
  }

  walk(root, "", 0);
  return tracks;
}

module.exports = {
  MATERIALIZED_KEYS,
  KNOWN_MATERIALIZED_AUDIO_KEYS,
  MATERIALIZED_METADATA_KEYS,
  eqBands,
  extractTracksFromObject,
  filterKeys,
  fxKeys,
  hasEqAutomation,
  hasFilterAutomation,
  hasFxAutomation,
  hasMaterializedEdge,
  transitionFromTrackPair,
  unknownAudioFields,
};

