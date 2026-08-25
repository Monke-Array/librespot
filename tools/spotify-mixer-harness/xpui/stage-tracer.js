(() => {
  const FORMAT = "spotify-mixer-stage-trace-v1";
  const AUTOMIX_SERVICE = "spotify.automix.esperanto.proto.Automix";
  const CONTEXT_PLAYER_SERVICE = "spotify.player.esperanto.proto.ContextPlayer";
  const CONTEXT_TRANSPORT_HOOK_VERSION = 2;
  const SENSITIVE_KEY = /authorization|cookie|credential|oauth|password|secret|session|token|email|account/i;
  const MIXER_KEY = /^(?:audio\.|automix\.|mix$|mix-|mixer_enabled|has-custom-transitions|can-view-transition|playlist\.revision)/;
  const CONTROL_FEATURE_ID = "spotify_mixer_stage_tracer";
  const RAW_MIXER_FIELD = /\b(?:audio|automix)\.[A-Za-z0-9_.-]+/g;
  const RAW_NEEDLES = [
    "audio.",
    "audio.fade_in_start_time",
    "audio.fade_in_duration",
    "audio.fade_out_start_time",
    "audio.fade_out_duration",
    "audio.fade_overlap",
    "audio.fade_in_curves",
    "audio.fade_out_curves",
    "audio.speed_automation",
    "audio.fade_in_eq_low_gain_curves",
    "audio.fade_out_eq_low_gain_curves",
    "audio.fade_in_filter_cutoff_curves",
    "audio.fade_out_filter_cutoff_curves",
    "audio.fade_in_filter_resonance_curves",
    "audio.fade_out_filter_resonance_curves",
    "automix.auto_preset_id",
    "automix.transition_uri",
    "automix.fade_in_cuepoint",
    "automix.fade_out_cuepoint",
  ];

  const textEncoder = new TextEncoder();

  const bytesToBase64 = (value) => {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value ?? []);
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 32768) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
    }
    return btoa(binary);
  };

  const bytesOf = (value) => {
    if (value instanceof Uint8Array) return value;
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    return new Uint8Array(value ?? []);
  };

  const sanitizeSnippet = (value) => String(value)
    .replace(/\bBearer\s+[-._~+/=A-Za-z0-9]+/g, "Bearer [REDACTED]")
    .replace(/(authorization|cookie|credential|oauth|password|secret|session|token|email|account)[^,\n\r\t ]*/gi, "$1[REDACTED]");

  const rawNeedleHits = (text) => Object.fromEntries(RAW_NEEDLES.map((needle) => {
    const positions = [];
    let cursor = 0;
    while (positions.length < 5) {
      const index = text.indexOf(needle, cursor);
      if (index < 0) break;
      positions.push(index);
      cursor = index + needle.length;
    }
    return [needle, {
      present: positions.length > 0,
      positions,
      snippets: positions.slice(0, 2).map((position) =>
        sanitizeSnippet(text.slice(Math.max(0, position - 64), position + needle.length + 96))
      ),
    }];
  }));

  const rawMixerFieldNames = (text) => [...new Set(text.match(RAW_MIXER_FIELD) ?? [])].sort();

  const rawTrackSegment = (text, uri) => {
    if (!uri) return { found: false, segment: "" };
    const start = text.indexOf(uri);
    if (start < 0) return { found: false, segment: "" };
    const nextTrack = text.indexOf("spotify:track:", start + uri.length);
    const end = nextTrack >= 0 ? nextTrack : Math.min(text.length, start + 20000);
    return {
      found: true,
      start,
      end,
      segment: text.slice(start, end),
    };
  };

  const scanRawEdge = (text, edge) => {
    const outgoing = rawTrackSegment(text, edge?.outgoingUri);
    const incoming = rawTrackSegment(text, edge?.incomingUri);
    const outgoingKeys = rawMixerFieldNames(outgoing.segment).filter(isOutgoingKey).sort();
    const incomingKeys = rawMixerFieldNames(incoming.segment).filter(isIncomingKey).sort();
    const directionalKeys = [...new Set([...outgoingKeys, ...incomingKeys])].sort();
    return {
      outgoing: {
        uri: edge?.outgoingUri ?? null,
        found: outgoing.found,
        start: outgoing.found ? outgoing.start : null,
        end: outgoing.found ? outgoing.end : null,
        keys: outgoingKeys,
      },
      incoming: {
        uri: edge?.incomingUri ?? null,
        found: incoming.found,
        start: incoming.found ? incoming.start : null,
        end: incoming.found ? incoming.end : null,
        keys: incomingKeys,
      },
      keys: directionalKeys,
      groups: groupsForKeys(directionalKeys),
    };
  };

  const scanRawBytes = (value, edge = null) => {
    const bytes = bytesOf(value);
    const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    const fieldNames = rawMixerFieldNames(text);
    const groups = groupsForKeys(fieldNames);
    const output = {
      byteLength: bytes.byteLength,
      groups,
      mixerFieldNames: fieldNames.slice(0, 120),
      needleHits: rawNeedleHits(text),
    };
    if (edge?.outgoingUri && edge?.incomingUri) {
      output.edgeDirectional = scanRawEdge(text, edge);
    }
    return output;
  };

  const decodeEsperantoEnvelope = (value) => {
    const bytes = bytesOf(value);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const decoder = new TextDecoder("utf-8", { fatal: false });
    let offset = 0;
    const readBytes = () => {
      if (offset + 4 > bytes.byteLength) throw new Error("truncated length prefix");
      const length = view.getInt32(offset);
      offset += 4;
      if (length < 0 || offset + length > bytes.byteLength) throw new Error("invalid length prefix");
      const part = bytes.slice(offset, offset + length);
      offset += length;
      return part;
    };
    try {
      const service = decoder.decode(readBytes());
      const method = decoder.decode(readBytes());
      const payload = readBytes();
      return {
        service,
        method,
        payloadByteLength: payload.byteLength,
        payloadScan: scanRawBytes(payload),
        requestByteLength: bytes.byteLength,
      };
    } catch (error) {
      return {
        service: null,
        method: null,
        requestByteLength: bytes.byteLength,
        error: String(error),
      };
    }
  };

  const normalize = (value, depth = 0, seen = new WeakSet()) => {
    if (value === null || value === undefined) return value ?? null;
    if (typeof value === "bigint") return value.toString();
    if (typeof value === "string") return value.replace(/\bBearer\s+[-._~+/=A-Za-z0-9]+/g, "Bearer [REDACTED]");
    if (typeof value === "number" || typeof value === "boolean") return value;
    if (typeof value === "function") return `[Function ${value.name || "anonymous"}]`;
    if (value instanceof Uint8Array || value instanceof ArrayBuffer) return { type: "bytes", base64: bytesToBase64(value) };
    if (seen.has(value)) return "[Circular]";
    if (depth > 10) return "[MaxDepth]";
    seen.add(value);
    if (Array.isArray(value)) return value.slice(0, 40).map((entry) => normalize(entry, depth + 1, seen));
    const output = {};
    for (const [key, entry] of Object.entries(value)) {
      output[key] = SENSITIVE_KEY.test(key) ? "[REDACTED]" : normalize(entry, depth + 1, seen);
    }
    return output;
  };

  const selectedMixerMetadata = (metadata) => Object.fromEntries(
    Object.entries(metadata ?? {})
      .filter(([key]) => MIXER_KEY.test(key))
      .map(([key, value]) => [key, normalize(value)]),
  );

  const keysOf = (metadata) => Object.keys(metadata ?? {}).sort();

  const hasAny = (keys, predicate) => keys.some(predicate);

  const groupsForKeys = (keys) => ({
    sparseAutomix: hasAny(keys, (key) =>
      key.startsWith("automix.") && key !== "automix.auto_preset_id"
    ),
    audio: hasAny(keys, (key) => key.startsWith("audio.")),
    presetId: keys.includes("automix.auto_preset_id"),
    transitionUri: keys.includes("automix.transition_uri"),
    cuepoints: hasAny(keys, (key) => key.startsWith("automix.fade_")),
    timing: hasAny(keys, (key) =>
      /^audio\.fade_(?:in|out)_(?:start_time|duration)$/.test(key) ||
      key === "audio.fade_overlap"
    ),
    volume: hasAny(keys, (key) => /^audio\.fade_(?:in|out)_curves$/.test(key)),
    speed: keys.includes("audio.speed_automation"),
    eq: hasAny(keys, (key) => key.includes("_eq_")),
    filter: hasAny(keys, (key) => key.includes("_filter_")),
    fx: hasAny(keys, (key) => /^audio\.(?:fx|echo|reverb)/.test(key)),
    unknownAudio: hasAny(keys, (key) =>
      key.startsWith("audio.") &&
      !/^audio\.fade_(?:in|out)_(?:start_time|duration|curves)$/.test(key) &&
      key !== "audio.fade_overlap" &&
      key !== "audio.speed_automation" &&
      key !== "audio.automix_mode" &&
      key !== "audio.only_allow_fade_on_advance" &&
      !key.includes("_eq_") &&
      !key.includes("_filter_")
    ),
  });

  const summarizeMetadata = (metadata) => {
    const fields = selectedMixerMetadata(metadata);
    const keys = keysOf(fields);
    return {
      keys,
      groups: groupsForKeys(keys),
      fields,
    };
  };

  const trackMetadata = (track) =>
    track?.metadata ??
    track?.contextTrack?.metadata ??
    track?.track?.metadata ??
    {};

  const summarizeTrack = (track) => {
    const metadata = trackMetadata(track);
    const summary = summarizeMetadata(metadata);
    return {
      uri:
        track?.uri ??
        track?.contextTrack?.uri ??
        track?.track?.uri ??
        metadata.uri ??
        null,
      uid:
        track?.uid ??
        track?.contextTrack?.uid ??
        track?.rowId ??
        track?.row_id ??
        null,
      name:
        track?.name ??
        track?.contextTrack?.name ??
        track?.track?.name ??
        metadata.title ??
        null,
      provider: track?.provider ?? null,
      metadataKeys: summary.keys,
      groups: summary.groups,
      metadata: summary.fields,
    };
  };

  const filterFields = (metadata, predicate) => Object.fromEntries(
    Object.entries(metadata ?? {}).filter(([key]) => predicate(key)),
  );

  const isOutgoingKey = (key) =>
    key.startsWith("audio.fade_out_") ||
    key === "audio.only_allow_fade_on_advance" ||
    key.startsWith("automix.fade_out_cuepoint.") ||
    key === "automix.transition_uri" ||
    key === "automix.auto_preset_id" ||
    key === "automix.mode";

  const isIncomingKey = (key) =>
    key.startsWith("audio.fade_in_") ||
    key === "audio.fade_overlap" ||
    key === "audio.speed_automation" ||
    key === "audio.automix_mode" ||
    key.startsWith("automix.fade_in_cuepoint.") ||
    key === "automix.transition_uri" ||
    key === "automix.auto_preset_id";

  const summarizeEdge = (current, next) => {
    const outgoing = summarizeTrack(current);
    const incoming = summarizeTrack(next);
    const outgoingFields = filterFields(outgoing.metadata, isOutgoingKey);
    const incomingFields = filterFields(incoming.metadata, isIncomingKey);
    const directionalKeys = [...Object.keys(outgoingFields), ...Object.keys(incomingFields)].sort();
    return {
      outgoingUri: outgoing.uri,
      incomingUri: incoming.uri,
      outgoingUid: outgoing.uid,
      incomingUid: incoming.uid,
      current: outgoing,
      next: incoming,
      materializedForEdge: {
        outgoing: {
          uri: outgoing.uri,
          fields: outgoingFields,
          keys: Object.keys(outgoingFields).sort(),
        },
        incoming: {
          uri: incoming.uri,
          fields: incomingFields,
          keys: Object.keys(incomingFields).sort(),
        },
      },
      groups: groupsForKeys(directionalKeys),
    };
  };

  const summarizeContext = (stateLike) => {
    const metadata = stateLike?.context?.metadata ?? stateLike?.contextMetadata ?? {};
    const summary = summarizeMetadata(metadata);
    return {
      uri: stateLike?.context?.uri ?? stateLike?.contextUri ?? null,
      metadataKeys: summary.keys,
      groups: summary.groups,
      metadata: summary.fields,
    };
  };

  const summarizePlayerState = (playerApi) => {
    const state = playerApi?.getState?.();
    const queue = playerApi?.getQueue?.();
    const current = queue?.current ?? state?.item ?? null;
    const nextItems = Array.isArray(queue?.nextUp)
      ? queue.nextUp
      : Array.isArray(state?.nextItems)
        ? state.nextItems
        : [];
    return {
      context: summarizeContext(state),
      state: {
        positionAsOfTimestamp: normalize(state?.positionAsOfTimestamp ?? null),
        duration: normalize(state?.duration ?? null),
        isPaused: state?.isPaused ?? null,
        playbackId: state?.playbackId ?? null,
        queueRevision: queue?.queueRevision ?? null,
      },
      edge: summarizeEdge(current, nextItems[0]),
      queuePreview: {
        current: summarizeTrack(current),
        nextUp: nextItems.slice(0, 5).map(summarizeTrack),
      },
    };
  };

  const summarizeContextPlayerState = (message) => ({
    context: summarizeContext(message),
    state: {
      positionAsOfTimestamp: normalize(message?.positionAsOfTimestamp ?? null),
      duration: normalize(message?.duration ?? null),
      isPaused: message?.isPaused ?? null,
      isPlaying: message?.isPlaying ?? null,
      playbackId: message?.playbackId ?? null,
      queueRevision: normalize(message?.queueRevision ?? null),
    },
    edge: summarizeEdge(message?.track, message?.nextTracks?.[0]),
    queuePreview: {
      current: summarizeTrack(message?.track),
      nextUp: (message?.nextTracks ?? []).slice(0, 5).map(summarizeTrack),
      previous: (message?.prevTracks ?? []).slice(0, 3).map(summarizeTrack),
    },
  });

  const summarizeContextPlayerQueue = (message) => ({
    queueRevision: normalize(message?.queueRevision ?? null),
    edge: summarizeEdge(message?.track, message?.nextTracks?.[0]),
    queuePreview: {
      current: summarizeTrack(message?.track),
      nextUp: (message?.nextTracks ?? []).slice(0, 5).map(summarizeTrack),
      previous: (message?.prevTracks ?? []).slice(0, 3).map(summarizeTrack),
    },
  });

  const edgeKey = (edge) => edge?.outgoingUri && edge?.incomingUri
    ? `${edge.outgoingUri}->${edge.incomingUri}`
    : null;

  const STAGE_RANK = {
    "context-player-native-bridge-request": 5,
    "context-player-native-bridge-response-raw": 6,
    "context-player-transport-request": 7,
    "context-player-transport-response-raw": 8,
    "context-player-state-stream": 10,
    "context-player-queue-stream": 20,
    "automix-method-request": 30,
    "automix-transport-request": 31,
    "automix-method-response": 40,
    "automix-transport-response": 41,
    "player-api-event": 50,
    "player-api-event-sync": 51,
    "player-api-snapshot": 60,
  };

  const buildSummary = (records) => {
    const edges = {};
    for (const record of records) {
      const key = edgeKey(record.edge);
      if (!key) continue;
      const entry = edges[key] ??= {
        outgoingUri: record.edge.outgoingUri,
        incomingUri: record.edge.incomingUri,
        stages: {},
        firstAudioStage: null,
        firstPresetStage: null,
        firstTimingStage: null,
        firstVolumeStage: null,
        firstSpeedStage: null,
        firstEqStage: null,
        firstFilterStage: null,
        firstStageRanks: {},
      };
      entry.stages[record.stage] = record.edge.groups;
      const setFirst = (property, group) => {
        if (!record.edge.groups?.[group]) return;
        const rank = STAGE_RANK[record.stage] ?? (1000 + record.sequence);
        if (entry.firstStageRanks[property] === undefined || rank < entry.firstStageRanks[property]) {
          entry.firstStageRanks[property] = rank;
          entry[property] = record.stage;
        }
      };
      setFirst("firstAudioStage", "audio");
      setFirst("firstPresetStage", "presetId");
      setFirst("firstTimingStage", "timing");
      setFirst("firstVolumeStage", "volume");
      setFirst("firstSpeedStage", "speed");
      setFirst("firstEqStage", "eq");
      setFirst("firstFilterStage", "filter");
    }
    for (const entry of Object.values(edges)) delete entry.firstStageRanks;
    return {
      records: records.length,
      automixInvocations: records.filter((record) => record.stage.startsWith("automix-")).length,
      edges: Object.values(edges),
    };
  };

  const installRspackRequire = () => {
    if (window.__spotifyRspackRequire) return window.__spotifyRspackRequire;
    const chunkName = Object.keys(window).find((key) => key.startsWith("rspackChunk"));
    if (!chunkName) throw new Error("Spotify Rspack runtime is unavailable");
    window[chunkName].push([[987654335], {}, (require) => {
      window.__spotifyRspackRequire = require;
    }]);
    return window.__spotifyRspackRequire;
  };

  const findModule = (modules, predicate) => {
    const match = Object.entries(modules).find(([, factory]) => predicate(String(factory)));
    return match ? Number(match[0]) : null;
  };

  const getPlayerApi = () => {
    window.spotifyMixerOracle?.controls?.status?.();
    const playerApi = window.__spotifyMixerOraclePlayerAPI;
    if (!playerApi) throw new Error("official PlayerAPI is unavailable; install mixer oracle first");
    return playerApi;
  };

  const edgeIdentityWithGroups = (edge, groups) => ({
    outgoingUri: edge?.outgoingUri ?? null,
    incomingUri: edge?.incomingUri ?? null,
    outgoingUid: edge?.outgoingUid ?? null,
    incomingUid: edge?.incomingUid ?? null,
    groups: { ...(groups ?? {}) },
  });

  const existing = window.spotifyMixerStageTracer;
  const tracer = existing ?? {
    format: FORMAT,
    installedAt: new Date().toISOString(),
    records: [],
    handles: [],
    hooks: {},
  };

  const record = (stage, payload = {}) => {
    const entry = {
      sequence: tracer.records.length,
      capturedAt: new Date().toISOString(),
      stage,
      ...payload,
    };
    tracer.records.push(normalize(entry));
    return entry;
  };

  const installAutomixHooks = () => {
    const modules = window.__webpack_modules__ ?? {};
    const require = installRspackRequire();
    const automixModuleId = findModule(modules, (source) =>
      source.includes(AUTOMIX_SERVICE) && source.includes("GetComputedTransitions")
    );
    const transportModuleId = findModule(modules, (source) =>
      source.includes("executeEsperantoCall") && source.includes("cancelEsperantoCall")
    );
    if (automixModuleId === null || transportModuleId === null) {
      tracer.hooks.automix = { installed: false, reason: "required modules not found" };
      return tracer.hooks.automix;
    }

    const Automix = require(automixModuleId).N5;
    const Transport = require(transportModuleId).h6;
    const playerEdge = () => summarizePlayerState(getPlayerApi()).edge;

    if (!Automix.prototype.__spotifyStageTracerOriginalGetComputedTransitions) {
      Object.defineProperty(Automix.prototype, "__spotifyStageTracerOriginalGetComputedTransitions", {
        value: Automix.prototype.getComputedTransitions,
      });
      Automix.prototype.getComputedTransitions = function (request, options) {
        const started = performance.now();
        record("automix-method-request", {
          edge: playerEdge(),
          service: AUTOMIX_SERVICE,
          method: "GetComputedTransitions",
          request: normalize(request),
        });
        return Automix.prototype.__spotifyStageTracerOriginalGetComputedTransitions.call(this, request, options).then(
          (response) => {
            record("automix-method-response", {
              edge: playerEdge(),
              service: AUTOMIX_SERVICE,
              method: "GetComputedTransitions",
              elapsedMs: performance.now() - started,
              response: normalize(response),
            });
            return response;
          },
          (error) => {
            record("automix-method-error", {
              edge: playerEdge(),
              service: AUTOMIX_SERVICE,
              method: "GetComputedTransitions",
              elapsedMs: performance.now() - started,
              error: String(error),
            });
            throw error;
          },
        );
      };
    }

    if (!Transport.prototype.__spotifyStageTracerOriginalCallSingle) {
      Object.defineProperty(Transport.prototype, "__spotifyStageTracerOriginalCallSingle", {
        value: Transport.prototype.callSingle,
      });
      Transport.prototype.callSingle = function (request, options) {
        if (request?.service !== AUTOMIX_SERVICE) {
          return Transport.prototype.__spotifyStageTracerOriginalCallSingle.call(this, request, options);
        }
        const decoder = Automix.DECODERS?.[request.method];
        const started = performance.now();
        const payload = request?.payload ?? textEncoder.encode("");
        const entry = record("automix-transport-request", {
          edge: playerEdge(),
          service: request.service,
          method: request.method,
          requestBase64: bytesToBase64(payload),
          request: decoder?.request ? normalize(decoder.request(payload)) : null,
        });
        return Transport.prototype.__spotifyStageTracerOriginalCallSingle.call(this, request, options).then(
          (response) => {
            record("automix-transport-response", {
              edge: playerEdge(),
              service: request.service,
              method: request.method,
              requestSequence: entry.sequence,
              elapsedMs: performance.now() - started,
              responseBase64: bytesToBase64(response),
              response: decoder?.response ? normalize(decoder.response(response)) : null,
            });
            return response;
          },
          (error) => {
            record("automix-transport-error", {
              edge: playerEdge(),
              service: request.service,
              method: request.method,
              requestSequence: entry.sequence,
              elapsedMs: performance.now() - started,
              error: String(error),
            });
            throw error;
          },
        );
      };
    }

    tracer.hooks.automix = {
      installed: true,
      automixModuleId,
      transportModuleId,
      methods: Object.keys(Automix.DECODERS ?? {}),
    };
    return tracer.hooks.automix;
  };

  const installContextPlayerTransportHook = () => {
    const modules = window.__webpack_modules__ ?? {};
    const require = installRspackRequire();
    const transportModuleId = findModule(modules, (source) =>
      source.includes("executeEsperantoCall") && source.includes("cancelEsperantoCall")
    );
    if (transportModuleId === null) {
      tracer.hooks.contextPlayerTransport = { installed: false, reason: "Esperanto transport module not found" };
      return tracer.hooks.contextPlayerTransport;
    }

    const Transport = require(transportModuleId).h6;
    const playerEdge = () => summarizePlayerState(getPlayerApi()).edge;
    const isGetQueue = (request) =>
      request?.service === CONTEXT_PLAYER_SERVICE && request?.method === "GetQueue";

    if (Transport.prototype.__spotifyStageTracerCallHookVersion !== CONTEXT_TRANSPORT_HOOK_VERSION) {
      const originalCall = Transport.prototype.__spotifyStageTracerOriginalCall ?? Transport.prototype.call;
      if (!Transport.prototype.__spotifyStageTracerOriginalCall) {
        Object.defineProperty(Transport.prototype, "__spotifyStageTracerOriginalCall", {
          value: originalCall,
        });
      }
      Transport.prototype.call = originalCall;
      Transport.prototype.call = function (request, persistent, onSuccess, onFailure) {
        if (!isGetQueue(request)) {
          return Transport.prototype.__spotifyStageTracerOriginalCall.call(this, request, persistent, onSuccess, onFailure);
        }

        const requestPayload = request?.payload ?? textEncoder.encode("");
        const requestScan = scanRawBytes(requestPayload);
        const requestRecord = record("context-player-transport-request", {
          edge: edgeIdentityWithGroups(playerEdge(), requestScan.groups),
          service: request.service,
          method: request.method,
          persistent: Boolean(persistent),
          payload: requestScan,
        });

        return Transport.prototype.__spotifyStageTracerOriginalCall.call(
          this,
          request,
          persistent,
          (response) => {
            const currentEdge = playerEdge();
            const rawResponse = scanRawBytes(response, currentEdge);
            const edgeGroups = rawResponse.edgeDirectional?.groups ?? rawResponse.groups;
            record("context-player-transport-response-raw", {
              edge: edgeIdentityWithGroups(currentEdge, edgeGroups),
              service: request.service,
              method: request.method,
              persistent: Boolean(persistent),
              requestSequence: requestRecord.sequence,
              rawResponse,
            });
            return onSuccess(response);
          },
          onFailure,
        );
      };
      Object.defineProperty(Transport.prototype, "__spotifyStageTracerCallHookVersion", {
        value: CONTEXT_TRANSPORT_HOOK_VERSION,
        configurable: true,
      });
    }

    let nativeBridgeHook = false;
    let nativeBridgeReason = null;
    try {
      const transport = getPlayerApi()?._contextPlayer?.transport;
      if (!transport || typeof transport._onSend !== "function") {
        nativeBridgeReason = "ContextPlayer transport _onSend is unavailable";
      } else if (transport.__spotifyStageTracerOnSendHookVersion === CONTEXT_TRANSPORT_HOOK_VERSION) {
        nativeBridgeHook = true;
      } else {
        const originalOnSend = transport.__spotifyStageTracerOriginalOnSend ?? transport._onSend;
        if (!transport.__spotifyStageTracerOriginalOnSend) {
          Object.defineProperty(transport, "__spotifyStageTracerOriginalOnSend", {
            value: originalOnSend,
          });
        }
        transport._onSend = originalOnSend;
        transport._onSend = function (envelope) {
          const decodedRequest = decodeEsperantoEnvelope(envelope?.request);
          if (decodedRequest.service !== CONTEXT_PLAYER_SERVICE || decodedRequest.method !== "GetQueue") {
            return originalOnSend.call(this, envelope);
          }

          const requestRecord = record("context-player-native-bridge-request", {
            edge: edgeIdentityWithGroups(playerEdge(), decodedRequest.payloadScan?.groups ?? groupsForKeys([])),
            service: decodedRequest.service,
            method: decodedRequest.method,
            persistent: Boolean(envelope?.persistent),
            serializedRequest: decodedRequest,
          });

          const wrappedEnvelope = {
            ...envelope,
            onSuccess: (response) => {
              const currentEdge = playerEdge();
              const rawResponse = scanRawBytes(response, currentEdge);
              const edgeGroups = rawResponse.edgeDirectional?.groups ?? rawResponse.groups;
              record("context-player-native-bridge-response-raw", {
                edge: edgeIdentityWithGroups(currentEdge, edgeGroups),
                service: decodedRequest.service,
                method: decodedRequest.method,
                persistent: Boolean(envelope?.persistent),
                requestSequence: requestRecord.sequence,
                rawResponse,
              });
              return envelope.onSuccess(response);
            },
          };

          return originalOnSend.call(this, wrappedEnvelope);
        };
        Object.defineProperty(transport, "__spotifyStageTracerOnSendHookVersion", {
          value: CONTEXT_TRANSPORT_HOOK_VERSION,
          configurable: true,
        });
        nativeBridgeHook = true;
      }
    } catch (error) {
      nativeBridgeReason = String(error);
    }

    tracer.hooks.contextPlayerTransport = {
      installed: true,
      transportModuleId,
      callHook: true,
      nativeBridgeHook,
      nativeBridgeReason,
    };
    return tracer.hooks.contextPlayerTransport;
  };

  const installPlayerEventHook = () => {
    const playerApi = getPlayerApi();
    const emitter = playerApi?._events?._emitter;
    if (!emitter) {
      tracer.hooks.playerEvents = { installed: false, reason: "PlayerAPI emitter not found" };
      return tracer.hooks.playerEvents;
    }
    if (!emitter.__spotifyStageTracerOriginalEmit) {
      Object.defineProperty(emitter, "__spotifyStageTracerOriginalEmit", { value: emitter.emit });
      emitter.emit = function (eventName, ...args) {
        if (/^(?:update|queue_update|ready)$/.test(String(eventName))) {
          record("player-api-event", {
            eventName: String(eventName),
            edge: summarizePlayerState(playerApi).edge,
            player: summarizePlayerState(playerApi),
            argsShape: args.map((arg) => normalize(arg, 0)).slice(0, 2),
          });
        }
        return emitter.__spotifyStageTracerOriginalEmit.call(this, eventName, ...args);
      };
    }
    if (!emitter.__spotifyStageTracerOriginalEmitSync) {
      Object.defineProperty(emitter, "__spotifyStageTracerOriginalEmitSync", { value: emitter.emitSync });
      emitter.emitSync = function (eventName, ...args) {
        if (/^(?:update|queue_update|ready)$/.test(String(eventName))) {
          record("player-api-event-sync", {
            eventName: String(eventName),
            edge: summarizePlayerState(playerApi).edge,
            player: summarizePlayerState(playerApi),
            argsShape: args.map((arg) => normalize(arg, 0)).slice(0, 2),
          });
        }
        return emitter.__spotifyStageTracerOriginalEmitSync.call(this, eventName, ...args);
      };
    }
    tracer.hooks.playerEvents = { installed: true };
    return tracer.hooks.playerEvents;
  };

  const cancelStreams = () => {
    for (const handle of tracer.handles.splice(0)) {
      try {
        handle.cancel();
      } catch {
        // Stream cancellation is best effort.
      }
    }
  };

  tracer.start = () => {
    const playerApi = getPlayerApi();
    installAutomixHooks();
    installContextPlayerTransportHook();
    installPlayerEventHook();
    cancelStreams();
    tracer.handles.push(playerApi._contextPlayer.getState({}, (message) => {
      const payload = summarizeContextPlayerState(message);
      record("context-player-state-stream", {
        service: CONTEXT_PLAYER_SERVICE,
        method: "GetState",
        edge: payload.edge,
        contextPlayer: payload,
      });
    }));
    tracer.handles.push(playerApi._contextPlayer.getQueue({}, (message) => {
      const payload = summarizeContextPlayerQueue(message);
      record("context-player-queue-stream", {
        service: CONTEXT_PLAYER_SERVICE,
        method: "GetQueue",
        edge: payload.edge,
        contextPlayer: payload,
      });
    }));
    record("player-api-snapshot", {
      edge: summarizePlayerState(playerApi).edge,
      player: summarizePlayerState(playerApi),
    });
    return tracer.status();
  };

  tracer.stop = () => {
    cancelStreams();
    return tracer.status();
  };

  tracer.clear = () => {
    tracer.records.length = 0;
    return tracer.status();
  };

  tracer.snapshot = () => {
    const playerApi = getPlayerApi();
    record("player-api-snapshot", {
      edge: summarizePlayerState(playerApi).edge,
      player: summarizePlayerState(playerApi),
    });
    return tracer.records.at(-1);
  };

  tracer.status = () => ({
    format: tracer.format,
    installedAt: tracer.installedAt,
    hooks: tracer.hooks,
    activeStreams: tracer.handles.length,
    records: tracer.records.length,
    summary: buildSummary(tracer.records),
  });

  tracer.exportTrace = () => ({
    format: tracer.format,
    capturedAt: new Date().toISOString(),
    installedAt: tracer.installedAt,
    source: {
      client: "official Spotify Desktop XPUI via CDP",
      boundary: "XPUI generated ContextPlayer stream and Automix client hooks",
    },
    hooks: tracer.hooks,
    records: tracer.records,
    summary: buildSummary(tracer.records),
  });

  window.spotifyMixerStageTracer = tracer;
  installAutomixHooks();
  installContextPlayerTransportHook();
  installPlayerEventHook();

  return {
    installed: true,
    format: tracer.format,
    hooks: tracer.hooks,
    status: tracer.status(),
  };
})();
