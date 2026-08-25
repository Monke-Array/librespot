const assert = require("assert");
const fs = require("fs");
const path = require("path");
const test = require("node:test");
const vm = require("vm");
const { TextDecoder, TextEncoder } = require("util");

const STAGE_TRACER_PATH = path.join(__dirname, "..", "xpui", "stage-tracer.js");
const CONTEXT_PLAYER_SERVICE = "spotify.player.esperanto.proto.ContextPlayer";

function createRuntime({
  responseText = [
    "audio.fade_in_start_time",
    "audio.fade_out_duration",
    "automix.auto_preset_id",
    "automix.transition_uri",
  ].join("\n"),
} = {}) {
  class AutomixClient {}
  AutomixClient.DECODERS = {};
  AutomixClient.prototype.getComputedTransitions = () => Promise.resolve({});

  class Transport {
    call(request, persistent, onSuccess) {
      const response = new TextEncoder().encode(responseText);
      onSuccess(response.buffer);
      return { cancel() {} };
    }

    callSingle() {
      return Promise.resolve(new Uint8Array());
    }
  }

  const playerEdge = {
    current: {
      uri: "spotify:track:out",
      uid: "out-uid",
      metadata: {},
    },
    nextUp: [
      {
        uri: "spotify:track:in",
        uid: "in-uid",
        metadata: {},
      },
    ],
  };

  const playerApi = {
    getState: () => ({
      context: { uri: "spotify:playlist:test", metadata: { mix: "true" } },
      item: playerEdge.current,
      nextItems: playerEdge.nextUp,
    }),
    getQueue: () => playerEdge,
    _events: {
      _emitter: {
        emit() {},
        emitSync() {},
      },
    },
    _contextPlayer: {
      transport: new Transport(),
      getState: () => ({ cancel() {} }),
      getQueue: () => ({ cancel() {} }),
    },
  };

  const modules = {
    1: function automixModule() {
      return "spotify.automix.esperanto.proto.Automix GetComputedTransitions";
    },
    2: function transportModule() {
      return "executeEsperantoCall cancelEsperantoCall";
    },
  };

  const window = {
    __webpack_modules__: modules,
    __spotifyRspackRequire(id) {
      if (Number(id) === 1) return { N5: AutomixClient };
      if (Number(id) === 2) return { h6: Transport };
      throw new Error(`unexpected module ${id}`);
    },
    spotifyMixerOracle: {
      controls: {
        status() {
          window.__spotifyMixerOraclePlayerAPI = playerApi;
        },
      },
    },
    __spotifyMixerOraclePlayerAPI: playerApi,
  };

  return {
    Transport,
    context: {
      window,
      TextDecoder,
      TextEncoder,
      btoa(value) {
        return Buffer.from(value, "binary").toString("base64");
      },
      performance: { now: () => 0 },
    },
  };
}

test("stage tracer records raw ContextPlayer GetQueue response before protobuf decoding", () => {
  const source = fs.readFileSync(STAGE_TRACER_PATH, "utf8");
  const { context, Transport } = createRuntime();

  vm.runInNewContext(source, context);

  const transport = new Transport();
  transport.call(
    { service: CONTEXT_PLAYER_SERVICE, method: "GetQueue", payload: new Uint8Array() },
    true,
    () => {},
    () => {},
  );

  const record = context.window.spotifyMixerStageTracer.records.find(
    (entry) => entry.stage === "context-player-transport-response-raw",
  );

  assert.ok(record, "expected a raw transport response record");
  assert.strictEqual(record.rawResponse.groups.audio, true);
  assert.strictEqual(record.rawResponse.groups.presetId, true);
  assert.strictEqual(record.rawResponse.groups.transitionUri, true);
});

test("stage tracer does not attribute incoming row outgoing raw fields to the active edge", () => {
  const source = fs.readFileSync(STAGE_TRACER_PATH, "utf8");
  const { context, Transport } = createRuntime({
    responseText: [
      "spotify:track:out",
      "audio.fade_out_duration",
      "automix.auto_preset_id",
      "automix.transition_uri",
      "spotify:track:in",
      "audio.fade_in_start_time",
      "audio.fade_out_filter_cutoff_curves",
      "spotify:track:after",
    ].join("\n"),
  });

  vm.runInNewContext(source, context);

  const transport = new Transport();
  transport.call(
    { service: CONTEXT_PLAYER_SERVICE, method: "GetQueue", payload: new Uint8Array() },
    true,
    () => {},
    () => {},
  );

  const record = context.window.spotifyMixerStageTracer.records.find(
    (entry) => entry.stage === "context-player-transport-response-raw",
  );

  assert.ok(record, "expected a raw transport response record");
  assert.strictEqual(record.rawResponse.groups.filter, true);
  assert.strictEqual(record.edge.groups.filter, false);
  assert.strictEqual(record.edge.groups.audio, true);
});
