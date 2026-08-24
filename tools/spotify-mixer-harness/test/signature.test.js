const assert = require("assert");
const test = require("node:test");

const { buildDspSignature } = require("../lib/signature");

function transition(overrides = {}) {
  return {
    pair: {
      outgoingUri: overrides.outgoingUri ?? "spotify:track:a",
      incomingUri: overrides.incomingUri ?? "spotify:track:b",
    },
    materialized: {
      outgoing: {
        uri: overrides.outgoingUri ?? "spotify:track:a",
        fields: {
          "automix.auto_preset_id": "1",
          "automix.transition_uri": overrides.transitionUri ?? "spotify:transition:first:1",
          "audio.fade_out_duration": "6090",
          "audio.fade_out_curves": "[{\"start_point\":0,\"end_point\":1,\"fade_curve\":[{\"x\":0,\"y\":1},{\"x\":1,\"y\":0}]}]",
          "audio.fade_out_eq_low_gain_curves": "[{\"start_point\":0,\"end_point\":1,\"fade_curve\":[{\"x\":0,\"y\":0.5},{\"x\":1,\"y\":0}]}]",
        },
      },
      incoming: {
        uri: overrides.incomingUri ?? "spotify:track:b",
        fields: {
          "audio.fade_overlap": "6090",
          "audio.fade_in_duration": "5500",
          "audio.fade_in_curves": "[{\"start_point\":0,\"end_point\":1,\"fade_curve\":[{\"x\":0,\"y\":0},{\"x\":1,\"y\":1}]}]",
          "audio.speed_automation": overrides.speed ?? "[{\"from_position\":0,\"speed\":0.9},{\"from_position\":27763,\"speed\":1}]",
        },
      },
    },
  };
}

test("DSP signature dedupes by effect shape, not song pair or transition URI", () => {
  const first = buildDspSignature(transition());
  const second = buildDspSignature(
    transition({
      outgoingUri: "spotify:track:c",
      incomingUri: "spotify:track:d",
      transitionUri: "spotify:transition:second:2",
    }),
  );

  assert.strictEqual(first.hash, second.hash);
  assert.strictEqual(first.summary.presetId, "1");
  assert.deepStrictEqual(first.summary.eqBands, ["out.low"]);
  assert.strictEqual(first.summary.speedPointCount, 2);
});

test("DSP signature changes when speed automation shape changes", () => {
  const first = buildDspSignature(transition());
  const second = buildDspSignature(
    transition({
      speed: "[{\"from_position\":0,\"speed\":0.85},{\"from_position\":1000,\"speed\":1}]",
    }),
  );

  assert.notStrictEqual(first.hash, second.hash);
});

test("DSP signature records unknown materialized audio fields", () => {
  const item = transition();
  item.materialized.outgoing.fields["audio.fade_out_reverb_dry_wet_curves"] = "[]";
  item.materialized.incoming.fields["audio.some_future_field"] = "1";

  const signature = buildDspSignature(item);

  assert.deepStrictEqual(signature.summary.unknownAudioFields, [
    "audio.fade_out_reverb_dry_wet_curves",
    "audio.some_future_field",
  ]);
});

