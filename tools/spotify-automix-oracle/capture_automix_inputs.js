// Paste into Spotify Desktop DevTools, or evaluate with cdp-eval.ps1.
// The helper only requests metadata for an explicitly supplied track.
(() => {
  const modules = window.__webpack_modules__;
  const chunkName = Object.keys(window).find((key) => key.startsWith("rspackChunk"));
  if (!modules || !chunkName) throw new Error("Rspack runtime is unavailable");

  if (!window.__spotifyRspackRequire) {
    window[chunkName].push([[987654322], {}, (require) => {
      window.__spotifyRspackRequire = require;
    }]);
  }

  const require = window.__spotifyRspackRequire;
  const extensionClient = new (require(95422).sp)(require(88343).n1());
  const decodeKnownExtension = require(9861).iy;
  const webgate = require(22358).n.getInstance();
  const textDecoder = new TextDecoder();
  const extensionKinds = {
    TRACK_DESCRIPTOR: 6,
    CUEPOINTS: 28,
    BEATS: 217,
    VOCAL_ACTIVITY: 218,
    MIXABILITY: 219,
    AUDIO_ATTRIBUTES_V2: 222,
  };

  const bytesToBase64 = (value) => {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 32768) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
    }
    return btoa(binary);
  };

  class WireReader {
    constructor(bytes) {
      this.bytes = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      this.position = 0;
    }

    varint() {
      let result = 0n;
      let shift = 0n;
      for (;;) {
        if (this.position >= this.bytes.length) throw new Error("truncated varint");
        const byte = this.bytes[this.position++];
        result |= BigInt(byte & 0x7f) << shift;
        if ((byte & 0x80) === 0) return result;
        shift += 7n;
        if (shift > 70n) throw new Error("invalid varint");
      }
    }

    fixed32() {
      if (this.position + 4 > this.bytes.length) throw new Error("truncated fixed32");
      const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.position, 4);
      this.position += 4;
      return view.getFloat32(0, true);
    }

    fixed64() {
      if (this.position + 8 > this.bytes.length) throw new Error("truncated fixed64");
      const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.position, 8);
      this.position += 8;
      return view.getFloat64(0, true);
    }

    lengthDelimited() {
      const length = Number(this.varint());
      if (this.position + length > this.bytes.length) throw new Error("truncated bytes");
      const result = this.bytes.subarray(this.position, this.position + length);
      this.position += length;
      return result;
    }

    fields() {
      const result = [];
      while (this.position < this.bytes.length) {
        const tag = Number(this.varint());
        const number = tag >>> 3;
        const wire = tag & 7;
        let value;
        if (wire === 0) value = this.varint();
        else if (wire === 1) value = this.fixed64();
        else if (wire === 2) value = this.lengthDelimited();
        else if (wire === 5) value = this.fixed32();
        else throw new Error(`unsupported protobuf wire type ${wire}`);
        result.push({ number, wire, value });
      }
      return result;
    }
  }

  const fields = (bytes) => new WireReader(bytes).fields();
  const first = (values, number) => values.find((field) => field.number === number)?.value;
  const all = (values, number) => values.filter((field) => field.number === number).map((field) => field.value);
  const numberValue = (value) => value === undefined ? undefined : Number(value);
  const stringValue = (value) => value === undefined ? undefined : textDecoder.decode(value);

  const decodePackedVarints = (bytes) => {
    const reader = new WireReader(bytes);
    const result = [];
    while (reader.position < reader.bytes.length) result.push(Number(reader.varint()));
    return result;
  };

  const decodeDescriptor = (bytes) => {
    const message = fields(bytes);
    const types = [];
    for (const field of message.filter((candidate) => candidate.number === 3)) {
      if (field.wire === 0) types.push(Number(field.value));
      else if (field.wire === 2) types.push(...decodePackedVarints(field.value));
    }
    return {
      text: stringValue(first(message, 1)),
      weight: first(message, 2),
      types,
      conceptUri: stringValue(first(message, 4)),
      localizedTerm: stringValue(first(message, 5)),
    };
  };

  const decodeDescriptors = (bytes) => {
    const message = fields(bytes);
    return all(message, 1).map(decodeDescriptor);
  };

  const decodeCuepoint = (bytes) => {
    const message = fields(bytes);
    const origin = numberValue(first(message, 3)) ?? 0;
    return {
      positionMs: numberValue(first(message, 1)) ?? 0,
      tempoBpm: first(message, 2) ?? 0,
      origin,
      originName: origin === 0 ? "HUMAN" : origin === 1 ? "ML" : "UNKNOWN",
      confidence: first(message, 4) ?? 0,
    };
  };

  const decodeCuepoints = (bytes) => {
    const message = fields(bytes);
    const optional = (number) => {
      const value = first(message, number);
      return value === undefined ? null : decodeCuepoint(value);
    };
    return {
      bestFadeIn: optional(1),
      bestFadeOut: optional(2),
      fadeInCandidates: all(message, 3).map(decodeCuepoint),
      fadeOutCandidates: all(message, 4).map(decodeCuepoint),
    };
  };

  const decodeBeat = (bytes) => {
    const message = fields(bytes);
    return {
      timeSeconds: first(message, 1) ?? 0,
      durationSeconds: first(message, 2) ?? 0,
      value: numberValue(first(message, 3)) ?? 0,
      beatConfidence: first(message, 4) ?? 0,
      downbeatConfidence: first(message, 5) ?? 0,
    };
  };

  const decodeBeats = (bytes) => {
    const message = fields(bytes);
    return {
      beatsPerBar: numberValue(first(message, 1)) ?? 0,
      beats: all(message, 2).map(decodeBeat),
      beatsHash: stringValue(first(message, 3)) ?? null,
    };
  };

  const decodeVocalActivity = (bytes) => {
    const message = fields(bytes);
    const probabilities = first(message, 5) ?? new Uint8Array();
    return {
      sourceSampleRateHz: first(message, 1) ?? 0,
      smoothingWindowSize: numberValue(first(message, 2)) ?? 0,
      firstWindowSampleStart: numberValue(first(message, 3)) ?? 0,
      samplesBetweenWindows: numberValue(first(message, 4)) ?? 0,
      probabilityEncoding: "uint8_percent_0_to_100",
      probabilities: Array.from(probabilities),
    };
  };

  const decodeMixability = (bytes) => {
    const message = fields(bytes);
    return {
      mixable: (numberValue(first(message, 1)) ?? 0) !== 0,
      genreBasedBeatmatchability: first(message, 2) ?? 0,
    };
  };

  // Spotify.dll+0x1004DD4: keep value=1 records, require a valid 4-beat
  // successor, or accept the native 2/4 correction when its implied BPM is
  // in [70, 180]. The final downbeat has no successor evidence and is kept.
  const normalizeDownbeats = (decodedBeats, allowTwoFourTimeSignature = true) => {
    const beats = decodedBeats.beats;
    const downbeatIndexes = [];
    beats.forEach((beat, index) => {
      if (beat.value === 1) downbeatIndexes.push(index);
    });
    const output = [];
    for (let ordinal = 0; ordinal < downbeatIndexes.length; ordinal++) {
      const index = downbeatIndexes[ordinal];
      const nextIndex = downbeatIndexes[ordinal + 1];
      const secondIndex = downbeatIndexes[ordinal + 2];
      const timeMs = Math.round(Math.fround(beats[index].timeSeconds) * 1000);
      let accepted = nextIndex === undefined || nextIndex - index === 4;
      let reason = nextIndex === undefined ? "terminal" : accepted ? "four-beat-spacing" : null;

      if (!accepted && allowTwoFourTimeSignature) {
        if (secondIndex !== undefined && secondIndex - index === 4) {
          const secondTimeMs = Math.round(Math.fround(beats[secondIndex].timeSeconds) * 1000);
          const impliedBpm = 240000 / (secondTimeMs - timeMs);
          if (impliedBpm >= 70 && impliedBpm <= 180) {
            accepted = true;
            reason = "two-downbeats-in-four-beats";
          }
        }
        if (!accepted && nextIndex !== undefined && nextIndex - index === 2) {
          const nextTimeMs = Math.round(Math.fround(beats[nextIndex].timeSeconds) * 1000);
          const impliedBpm = 240000 / (nextTimeMs - timeMs);
          if (impliedBpm >= 70 && impliedBpm <= 180) {
            accepted = true;
            reason = "two-four-time-signature";
          }
        }
      }

      if (accepted) {
        output.push({
          rawBeatIndex: index,
          positionMs: timeMs,
          positionSeconds: Math.fround(timeMs * 0.001),
          confidence: Math.fround(
            Math.fround(beats[index].beatConfidence) * Math.fround(beats[index].downbeatConfidence),
          ),
          acceptanceReason: reason,
        });
      }
    }
    return output;
  };

  const simplifyHeader = (header) => ({
    statusCode: header?.statusCode ?? null,
    cacheValid: header?.cacheValid ?? null,
    offlineValid: header?.offlineValid ?? null,
    isEmpty: header?.isEmpty ?? null,
    etag: header?.etag ?? null,
  });

  const extensionByKind = (response, kind) => {
    const group = response.extension.find((entry) => entry.extensionKind === kind);
    if (!group || group.entityExtension.length !== 1) {
      throw new Error(`expected one extension result for kind ${kind}`);
    }
    return group.entityExtension[0];
  };

  const retainAutomixAnalysisFields = (body) => {
    const scalarObject = (value, omitted) => Object.fromEntries(
      Object.entries(value ?? {}).filter(([key, entry]) => !omitted.includes(key) && (
        entry === null || ["string", "number", "boolean"].includes(typeof entry)
      )),
    );
    const track = scalarObject(body.track, [
      "codestring", "echoprintstring", "synchstring", "rhythmstring",
    ]);
    for (const key of ["codestring", "echoprintstring", "synchstring", "rhythmstring"]) {
      if (key in (body.track ?? {})) {
        track[`${key}Length`] = typeof body.track[key] === "string" ? body.track[key].length : null;
      }
    }
    return {
      meta: scalarObject(body.meta, []),
      track,
      bars: (body.bars ?? []).map((bar) => scalarObject(bar, [])),
      segments: (body.segments ?? []).map((segment) => scalarObject(segment, ["pitches", "timbre"])),
      omittedAsUnusedByAutomix: {
        topLevel: ["beats", "sections", "tatums"],
        segmentFields: ["pitches", "timbre"],
        trackFingerprintStrings: ["codestring", "echoprintstring", "synchstring", "rhythmstring"],
      },
    };
  };

  const fetchAudioAnalysis = async (playableTrackUri) => {
    const trackId = playableTrackUri.split(":").at(-1);
    const response = await webgate.build()
      .withHost("https://spclient.wg.spotify.com")
      .withPath(`/audio-attributes/v1/audio-analysis/${trackId}`)
      .withEndpointIdentifier("/audio-attributes/v1/audio-analysis/{id}")
      .withoutMarket()
      .send();
    if (response.status !== 200) throw new Error(`audio analysis status ${response.status}`);
    return retainAutomixAnalysisFields(response.body);
  };

  window.spotifyAutomixInputCapture = {
    format: "spotify-automix-track-input-v1",
    extensionKinds,
    normalizeDownbeats,
    async captureTrack({ canonicalTrackUri, playableTrackUri, expectedBeatsHash = null }) {
      if (!canonicalTrackUri?.startsWith("spotify:track:") || !playableTrackUri?.startsWith("spotify:track:")) {
        throw new Error("canonicalTrackUri and playableTrackUri must be Spotify track URIs");
      }
      // Native Automix keeps editorial/cue metadata on the requested track,
      // but resolves timing/audio analysis against the playable substitute.
      const sourceUriByKind = {
        [extensionKinds.TRACK_DESCRIPTOR]: canonicalTrackUri,
        [extensionKinds.CUEPOINTS]: canonicalTrackUri,
        [extensionKinds.BEATS]: playableTrackUri,
        [extensionKinds.VOCAL_ACTIVITY]: playableTrackUri,
        [extensionKinds.MIXABILITY]: canonicalTrackUri,
        [extensionKinds.AUDIO_ATTRIBUTES_V2]: playableTrackUri,
      };
      const requestedKinds = Object.values(extensionKinds);
      const response = await extensionClient.fetch({
        extensionQuery: requestedKinds.map((extensionKind) => ({
          extensionKind,
          entityUri: [sourceUriByKind[extensionKind]],
        })),
      });

      const raw = {};
      for (const [name, kind] of Object.entries(extensionKinds)) {
        const result = extensionByKind(response, kind);
        const value = result.extensionData.value;
        raw[name] = {
          extensionKind: kind,
          requestedEntityUri: sourceUriByKind[kind],
          returnedEntityUri: result.entityUri,
          header: simplifyHeader(result.header),
          typeUrl: result.extensionData.typeUrl,
          protobufBase64: bytesToBase64(value),
        };
      }

      const bytes = (name) => extensionByKind(response, extensionKinds[name]).extensionData.value;
      const beats = decodeBeats(bytes("BEATS"));
      const audioAttributesV2 = decodeKnownExtension(extensionKinds.AUDIO_ATTRIBUTES_V2, bytes("AUDIO_ATTRIBUTES_V2"));
      const audioAnalysis = await fetchAudioAnalysis(playableTrackUri);
      const beatsHashMatchesExpected = expectedBeatsHash === null
        ? null
        : beats.beatsHash === expectedBeatsHash;

      return {
        format: this.format,
        capturedAt: new Date().toISOString(),
        source: {
          client: "official Spotify desktop",
          extensionService: "spotify.extendedmetadata.v1.ExtendedMetadata/Fetch",
          audioAnalysisUri: `hm://audio-attributes/v1/audio-analysis/${playableTrackUri.split(":").at(-1)}`,
          audioAnalysisTransport: "official in-process authenticated WebGate client",
        },
        identity: {
          canonicalTrackUri,
          playableTrackUri,
          playableSubstitution: canonicalTrackUri !== playableTrackUri,
          expectedPlayableBeatsHash: expectedBeatsHash,
          capturedBeatsHash: beats.beatsHash,
          beatsHashMatchesExpected,
        },
        duration: {
          seconds: audioAnalysis.track?.duration ?? null,
          source: "audio-analysis.track.duration",
        },
        rawExtensions: raw,
        decoded: {
          trackDescriptors: decodeDescriptors(bytes("TRACK_DESCRIPTOR")),
          cuepoints: decodeCuepoints(bytes("CUEPOINTS")),
          beats,
          normalizedDownbeats: normalizeDownbeats(beats, true),
          vocalActivity: decodeVocalActivity(bytes("VOCAL_ACTIVITY")),
          mixability: decodeMixability(bytes("MIXABILITY")),
          audioAttributesV2,
          audioAnalysis,
        },
        preprocessing: {
          downbeats: {
            evidence: "Spotify.dll+0x1004DD4",
            positionConversion: "round(float32(Beat.time_seconds) * 1000)",
            confidenceFormula: "float32(float32(beat_confidence) * float32(downbeat_confidence))",
            usesBeatValue: 1,
            allowTwoFourTimeSignature: true,
            impliedBpmInclusiveRange: [70, 180],
          },
        },
      };
    },
  };

  return {
    installed: true,
    format: window.spotifyAutomixInputCapture.format,
    extensionKinds,
  };
})();
