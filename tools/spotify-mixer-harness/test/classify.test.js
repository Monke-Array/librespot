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

test("classifier uses local rejection reason for timing and speed failures", () => {
  assert.strictEqual(
    classifyTransition({
      devEqBypass: true,
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
