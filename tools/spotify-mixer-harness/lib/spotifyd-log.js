"use strict";

function uniquePush(target, value) {
  if (!target.includes(value)) target.push(value);
}

function parseSpotifydLog(logText, options = {}) {
  const text = String(logText ?? "");
  const lines = text.split(/\r?\n/).filter(Boolean);
  const transitionLines = [];
  const spotifyMixLines = [];
  const rejectionReasons = [];
  const runtimeFailures = [];
  const events = [];
  let selectedPath = null;

  for (const line of lines) {
    if (line.includes("[transition]") || line.includes("Transition started")) {
      transitionLines.push(line);
    }
    if (line.includes("[spotify-mix]") || line.includes("[spotify-auto]")) {
      spotifyMixLines.push(line);
    }

    if (line.includes("[spotify-mix] materialized volume transition")) {
      selectedPath = "materialized";
      const match = line.match(/materialized volume transition\s+(spotify:track:[^\s]+)\s+->\s+(spotify:track:[^\s]+)/);
      events.push({
        selectedPath: "materialized",
        outgoingUri: match?.[1] ?? null,
        incomingUri: match?.[2] ?? null,
        line: line.trim(),
      });
    } else if (line.includes("[spotify-mix] saved transition") || line.includes("[spotify-mix] saved recipe decoded")) {
      selectedPath = selectedPath ?? "saved";
      const match = line.match(/saved transition\s+(spotify:track:[^\s]+)\s+->\s+(spotify:track:[^\s]+)/);
      if (match) {
        events.push({
          selectedPath: "saved",
          outgoingUri: match[1],
          incomingUri: match[2],
          line: line.trim(),
        });
      }
    } else if (line.includes("[spotify-auto] local Auto") || line.includes("[spotify-mix] local Auto transition")) {
      selectedPath = selectedPath ?? "local-auto";
    } else if (line.includes("using fallback")) {
      selectedPath = selectedPath ?? "fallback";
    }

    if (
      /unsupported:|invalid materialized transition|invalid saved transition|recipe unsupported|using fallback/i.test(line)
    ) {
      rejectionReasons.push(line.trim());
    }
    if (/secondary PCM underrun/i.test(line)) uniquePush(runtimeFailures, "secondary-underrun");
    if (/thread '.+' panicked|panicked at/i.test(line)) uniquePush(runtimeFailures, "panic");
  }

  if (options.exitCode !== undefined && options.exitCode !== null && options.exitCode !== 0) {
    uniquePush(runtimeFailures, "process-exit");
  }

  return {
    selectedPath,
    rejectionReasons,
    runtimeFailures,
    events,
    spotifyMixLines,
    transitionLines,
  };
}

module.exports = {
  parseSpotifydLog,
};
