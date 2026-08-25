const assert = require("assert");
const test = require("node:test");

const { buildCapabilityDiffReport, summarizeMixerEndpointStatus } = require("../lib/capabilities");

test("capability diff normalizes official desktop and spotifyd advertisements", () => {
  const logText = `
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] incoming source=initial-put-state-response device-index=0 device-info active=Some(false) can-play=true type=Ok(COMPUTER)(1) software-version=1.2.97.270.ge94a76a2 spirc-version=3.2.6 client-id=65b708073fc0480ea92a077233ca87bd product-id=<unset> brand=spotify model=PC desktop group=false dynamic=false social-connect=false private-session=false offline=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] incoming source=initial-put-state-response device-index=0 device-info sensitive-or-user-fields name-present=true device-id-present=true deduplication-id-present=false public-ip-present=true license-present=true aliases=0 selected-alias-id=0 metadata-keys=["debug_level", "tier1_port"] disallow-playback=[] disallow-transfer=[]
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] device-info unknown-protobuf-fields=["33:Varint"]
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] incoming source=initial-put-state-response device-index=0 capability-booleans can-be-player=true restrict-to-local=false gaia-eq-connect-id=true supports-logout=true observable=true command-acks=true supports-rename=false hidden=false disable-volume=false connect-disabled=false supports-playlist-v2=true controllable=true supports-external-episodes=true supports-set-backend-metadata=true supports-transfer-command=true supports-command-request=true voice-enabled=false needs-full-player-state=false supports-gzip-pushes=true supports-set-options-command=true supports-rooms=false supports-dj=true
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] incoming source=initial-put-state-response device-index=0 capability-values volume-steps=64 supported-media-types=["audio/ad", "audio/audio", "audio/media", "audio/track"] supported-audio-quality=Ok(HIFI)(5) connect-capabilities=<unset> supported-codecs=[] codec-field-present-in-connect-schema=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] incoming source=initial-put-state-response device-index=0 supports-hifi present=true fully-supported=true user-eligible=true device-supported=true
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] capabilities unknown-protobuf-fields=["33:Varint", "34:Varint", "35:Varint", "38:Varint"]
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] outgoing device-info active=None can-play=true type=Ok(SPEAKER)(4) software-version=0.8.0 spirc-version=3.2.6 client-id=65b708073fc0480ea92a077233ca87bd product-id=<unset> brand=<unset> model=<unset> group=false dynamic=false social-connect=false private-session=false offline=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] outgoing device-info sensitive-or-user-fields name-present=true device-id-present=true deduplication-id-present=false public-ip-present=false license-present=false aliases=0 selected-alias-id=0 metadata-keys=[] disallow-playback=[] disallow-transfer=[]
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] device-info unknown-protobuf-fields=[]
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] outgoing capability-booleans can-be-player=true restrict-to-local=false gaia-eq-connect-id=true supports-logout=false observable=true command-acks=true supports-rename=false hidden=false disable-volume=false connect-disabled=false supports-playlist-v2=true controllable=true supports-external-episodes=false supports-set-backend-metadata=false supports-transfer-command=true supports-command-request=true voice-enabled=false needs-full-player-state=true supports-gzip-pushes=true supports-set-options-command=true supports-rooms=false supports-dj=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] outgoing capability-values volume-steps=64 supported-media-types=["audio/episode", "audio/local", "audio/track"] supported-audio-quality=Ok(VERY_HIGH)(4) connect-capabilities=<unset> supported-codecs=[] codec-field-present-in-connect-schema=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] outgoing supports-hifi present=false
[DEBUG librespot_connect::capability_debug] [spotify-capability-debug] capabilities unknown-protobuf-fields=[]
`;

  const report = buildCapabilityDiffReport({ logText, generatedAt: "test" });

  assert.strictEqual(report.official.source, "incoming source=initial-put-state-response device-index=0");
  assert.strictEqual(report.spotifyd.source, "outgoing");
  assert.strictEqual(report.official.known.capabilities.supportsDj, true);
  assert.strictEqual(report.spotifyd.known.capabilities.supportsDj, false);
  assert.deepStrictEqual(report.official.unknown.capabilities, [
    "33:Varint",
    "34:Varint",
    "35:Varint",
    "38:Varint",
  ]);
  assert.deepStrictEqual(report.spotifyd.unknown.capabilities, []);

  const paths = report.differences.map((difference) => difference.path);
  assert(paths.includes("known.capabilities.supportsDj"));
  assert(paths.includes("known.capabilities.supportsSetBackendMetadata"));
  assert(paths.includes("known.capabilities.needsFullPlayerState"));
  assert(paths.includes("known.capabilities.supportedMediaTypes"));
  assert(paths.includes("unknown.capabilities"));
});

test("endpoint status summary records materialized audio and automix fields", () => {
  const summary = summarizeMixerEndpointStatus({
    officialPlayer: {
      context: {
        uri: "spotify:playlist:test",
        isMixerEnabled: true,
        metadata: {
          mix: "true",
          "automix.mode": "auto",
        },
      },
      state: {
        item: {
          uri: "spotify:track:a",
          name: "A",
          materializedKeys: ["audio.fade_out_curves", "automix.auto_preset_id", "automix.transition_uri"],
        },
        nextItems: [
          {
            uri: "spotify:track:b",
            name: "B",
            materializedKeys: ["audio.fade_in_curves"],
          },
        ],
      },
      materializedTransitionAvailable: true,
    },
  });

  assert.strictEqual(summary.contextUri, "spotify:playlist:test");
  assert.strictEqual(summary.isMixerEnabled, true);
  assert.strictEqual(summary.materializedTransitionAvailable, true);
  assert.strictEqual(summary.current.hasAudio, true);
  assert.strictEqual(summary.current.hasAutoPresetId, true);
  assert.strictEqual(summary.current.hasTransitionUri, true);
  assert.strictEqual(summary.next.hasAudio, true);
});
