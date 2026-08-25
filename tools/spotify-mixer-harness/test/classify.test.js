const assert = require("assert");
const test = require("node:test");

const { classifyTransition } = require("../lib/classify");
const { parseSpotifydLog } = require("../lib/spotifyd-log");

function materialized(fields) {
  return {
    outgoing: { fields: fields.outgoing ?? {} },
    incoming: { fields: fields.incoming ?? {} },
  };
}

test("classifier reports dev EQ bypass when only EQ blocks a selected materialized plan", () => {
  const classification = classifyTransition({
    devEqBypass: true,
    materialized: materialized({
      outgoing: {
        "audio.fade_out_eq_low_gain_curves": "[]",
      },
    }),
    localResult: {
      selectedPath: "materialized",
      runtimeFailures: [],
      rejectionReasons: [],
    },
  });

  assert.strictEqual(classification.status, "DEV-EQ-BYPASS");
  assert.deepStrictEqual(classification.blockers, []);
});

test("classifier keeps filter and FX failures ahead of EQ bypass", () => {
  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
      devFilterBypass: false,
      materialized: materialized({
        outgoing: {
          "audio.fade_out_eq_low_gain_curves": "[]",
          "audio.fade_out_filter_cutoff_curves": "[]",
        },
      }),
      localResult: {},
    }).status,
    "UNSUPPORTED-FILTER",
  );

  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
      devFilterBypass: true,
      materialized: materialized({
        outgoing: {
          "audio.fade_out_reverb_dry_wet_curves": "[]",
        },
      }),
      localResult: {},
    }).status,
    "UNSUPPORTED-FX",
  );
});

test("classifier reports dev filter bypass when filter is explicitly ignored", () => {
  const classification = classifyTransition({
    devFilterBypass: true,
    materialized: materialized({
      outgoing: {
        "audio.fade_out_filter_cutoff_curves": "[]",
        "audio.fade_out_filter_resonance_curves": "[]",
      },
    }),
    localResult: {},
  });

  assert.strictEqual(classification.status, "DEV-FILTER-BYPASS");
  assert.deepStrictEqual(classification.blockers, []);
});

test("classifier reports combined dev EQ and filter bypass only when both are enabled", () => {
  const bothBlocked = materialized({
    outgoing: {
      "audio.fade_out_eq_low_gain_curves": "[]",
      "audio.fade_out_filter_cutoff_curves": "[]",
    },
  });

  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
      devFilterBypass: false,
      materialized: bothBlocked,
      localResult: {},
    }).status,
    "UNSUPPORTED-FILTER",
  );

  const classification = classifyTransition({
    devEqBypass: true,
    devFilterBypass: true,
    materialized: bothBlocked,
    localResult: {},
  });

  assert.strictEqual(classification.status, "DEV-EQ-FILTER-BYPASS");
  assert.deepStrictEqual(classification.blockers, []);
});

test("classifier does not bypass unknown FX even when EQ and filter bypasses are enabled", () => {
  const classification = classifyTransition({
    devEqBypass: true,
    devFilterBypass: true,
    materialized: materialized({
      outgoing: {
        "audio.fade_out_eq_low_gain_curves": "[]",
        "audio.fade_out_filter_cutoff_curves": "[]",
        "audio.fade_out_echo_dry_wet_curves": "[]",
      },
    }),
    localResult: {},
  });

  assert.strictEqual(classification.status, "UNSUPPORTED-FX");
  assert.deepStrictEqual(classification.blockers, ["fx"]);
});

test("classifier uses local rejection reason for timing and speed failures", () => {
  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
      devFilterBypass: true,
      materialized: materialized({}),
      localResult: {
        rejectionReasons: [
          "materialized transition unsupported: materialized transition source durations do not match its wall-clock overlap; using fallback",
        ],
      },
    }).status,
    "UNSUPPORTED-TIMING",
  );

  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
      devFilterBypass: true,
      materialized: materialized({}),
      localResult: {
        rejectionReasons: [
          "materialized transition unsupported: materialized transition has unsupported speed automation; using fallback",
        ],
      },
    }).status,
    "UNSUPPORTED-SPEED",
  );
});

test("spotifyd log parser extracts selected path, fallbacks, and runtime failures", () => {
  const parsed = parseSpotifydLog(`
[DEBUG librespot_connect::spotify_mix] [spotify-mix] materialized volume transition spotify:track:a -> spotify:track:b
[DEBUG librespot_playback::transition] Transition started
[DEBUG librespot_playback::player] Transition aborted: secondary PCM underrun; continuing current track
thread 'main' panicked at src/main.rs:1:1
`);

  assert.strictEqual(parsed.selectedPath, "materialized");
  assert.deepStrictEqual(parsed.runtimeFailures, ["secondary-underrun", "panic"]);
  assert.strictEqual(parsed.transitionLines.length, 1);
  assert.deepStrictEqual(parsed.events, [
    {
      selectedPath: "materialized",
      outgoingUri: "spotify:track:a",
      incomingUri: "spotify:track:b",
      line: "[DEBUG librespot_connect::spotify_mix] [spotify-mix] materialized volume transition spotify:track:a -> spotify:track:b",
    },
  ]);
});
