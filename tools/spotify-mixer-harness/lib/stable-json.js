"use strict";

function stable(value) {
  if (Array.isArray(value)) return value.map(stable);
  if (!value || typeof value !== "object") return value;

  const output = {};
  for (const key of Object.keys(value).sort()) {
    if (value[key] !== undefined) output[key] = stable(value[key]);
  }
  return output;
}

function stableStringify(value) {
  return JSON.stringify(stable(value));
}

module.exports = {
  stable,
  stableStringify,
};

