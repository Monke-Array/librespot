const assert = require("assert");
const test = require("node:test");

const { DEFAULT_TEST_PAIR, buildControlExpression } = require("../bin/mixer-harness");

test("mute control expression does not require a URI", () => {
  assert.strictEqual(
    buildControlExpression({ action: "mute" }),
    "window.spotifyMixerOracle.controls.mute()",
  );
});

test("play-uri control expression still requires an explicit URI", () => {
  assert.throws(
    () => buildControlExpression({ action: "play-uri" }),
    /missing --uri/,
  );
});

test("status control expression reads official player state", () => {
  assert.strictEqual(
    buildControlExpression({ action: "status" }),
    "window.spotifyMixerOracle.controls.status()",
  );
});

test("play-context control expression requires a context URI and forwards seek options", () => {
  assert.throws(
    () => buildControlExpression({ action: "play-context" }),
    /missing --context-uri/,
  );

  assert.strictEqual(
    buildControlExpression({
      action: "play-context",
      contextUri: "spotify:playlist:test",
      skipUri: "spotify:track:one",
      seekMs: "1234",
      paused: true,
    }),
    "window.spotifyMixerOracle.controls.playContext({\"contextUri\":\"spotify:playlist:test\",\"skipUri\":\"spotify:track:one\",\"seekMs\":1234,\"paused\":true})",
  );
});

test("play-pair defaults to the harness-only known materialized transition pair", () => {
  assert.deepStrictEqual(DEFAULT_TEST_PAIR, {
    firstUri: "spotify:track:2BMRUAA1oTc7e9JPlr6xbZ",
    secondUri: "spotify:track:5g9lS8deSIxItFBmZRC4vN",
  });

  assert.strictEqual(
    buildControlExpression({ action: "play-pair" }),
    "window.spotifyMixerOracle.controls.playPair({\"firstUri\":\"spotify:track:2BMRUAA1oTc7e9JPlr6xbZ\",\"secondUri\":\"spotify:track:5g9lS8deSIxItFBmZRC4vN\"})",
  );
});

test("transfer control expression requires a target device selector", () => {
  assert.throws(
    () => buildControlExpression({ action: "transfer" }),
    /missing --device-name or --device-id/,
  );

  assert.strictEqual(
    buildControlExpression({ action: "transfer", deviceName: "spotifyd-transition DEV" }),
    "window.spotifyMixerOracle.controls.transfer({\"deviceName\":\"spotifyd-transition DEV\"})",
  );
});
