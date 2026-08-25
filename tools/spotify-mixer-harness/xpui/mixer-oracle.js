(() => {
  const SERVICE = "spotify.automix.esperanto.proto.Automix";
  const METHOD = "GetComputedTransitions";
  const SENSITIVE_KEY = /authorization|cookie|credential|oauth|password|secret|session|token/i;
  const MATERIALIZED_KEY = /^(?:audio\.|automix\.(?:auto_preset_id|mode|transition_uri|fade_(?:in|out)_cuepoint\.))/;
  const MIXER_CONTEXT_KEY = /^(?:automix\.|mix$|mix-|mixer_enabled|has-custom-transitions|can-view-transition|playlist\.revision)/;
  const CONTROL_FEATURE_ID = "spotify_mixer_harness";

  const textEncoder = new TextEncoder();
  const textDecoder = new TextDecoder();

  const bytesToBase64 = (value) => {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 32768) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
    }
    return btoa(binary);
  };

  const cloneSafe = (value, depth = 0, seen = new WeakSet()) => {
    if (value === null || typeof value !== "object") {
      if (typeof value === "string") return value.replace(/\bBearer\s+[-._~+/=A-Za-z0-9]+/g, "Bearer [REDACTED]");
      if (typeof value === "bigint") return value.toString();
      return value;
    }
    if (seen.has(value)) return "[Circular]";
    if (depth > 12) return "[MaxDepth]";
    seen.add(value);
    if (value instanceof Uint8Array || value instanceof ArrayBuffer) return bytesToBase64(value);
    if (Array.isArray(value)) return value.map((entry) => cloneSafe(entry, depth + 1, seen));
    const output = {};
    for (const [key, entry] of Object.entries(value)) {
      output[key] = SENSITIVE_KEY.test(key) ? "[REDACTED]" : cloneSafe(entry, depth + 1, seen);
    }
    return output;
  };

  const materializedFields = (metadata) => Object.fromEntries(
    Object.entries(metadata ?? {}).filter(([key]) => MATERIALIZED_KEY.test(key)),
  );

  const hasMaterializedFields = (metadata) => Object.keys(materializedFields(metadata)).length > 0;

  const mixerContextFields = (metadata) => Object.fromEntries(
    Object.entries(metadata ?? {}).filter(([key]) => MIXER_CONTEXT_KEY.test(key)),
  );

  const isMixerContext = (context) => {
    const metadata = context?.metadata ?? {};
    return ["automix.queue", "has-custom-transitions", "mix", "mixer_enabled", "can-view-transition"].some(
      (key) => metadata[key] === "true",
    );
  };

  const findModule = (modules, predicate) => {
    const match = Object.entries(modules).find(([, factory]) => predicate(String(factory)));
    return match ? Number(match[0]) : null;
  };

  const installRspackRequire = () => {
    if (window.__spotifyRspackRequire) return window.__spotifyRspackRequire;
    const chunkName = Object.keys(window).find((key) => key.startsWith("rspackChunk"));
    if (!chunkName) throw new Error("Spotify Rspack runtime is unavailable");
    window[chunkName].push([[987654333], {}, (require) => {
      window.__spotifyRspackRequire = require;
    }]);
    return window.__spotifyRspackRequire;
  };

  const methodNames = (value) => {
    if (!value || typeof value !== "object") return [];
    const names = new Set();
    let current = value;
    for (let depth = 0; current && depth < 5; depth++, current = Object.getPrototypeOf(current)) {
      for (const name of Object.getOwnPropertyNames(current)) {
        try {
          if (typeof value[name] === "function") names.add(name);
        } catch {
          // Ignore getters that throw while probing minified XPUI objects.
        }
      }
    }
    return [...names].sort();
  };

  const hasPlayerApiShape = (value) =>
    value &&
    typeof value === "object" &&
    typeof value.getState === "function" &&
    typeof value.getQueue === "function" &&
    typeof value.play === "function";

  const hasConnectApiShape = (value) =>
    value &&
    typeof value === "object" &&
    typeof value.getDevices === "function" &&
    (typeof value.transferPlayback === "function" ||
      typeof value.activateDevice === "function" ||
      typeof value.setActiveDevice === "function");

  const inspectReactFiberValues = (predicate, limit = 1) => {
    const matches = [];
    const seenFibers = new WeakSet();
    const seenValues = new WeakSet();

    const inspectValue = (value, path) => {
      if (!value || (typeof value !== "object" && typeof value !== "function")) return;
      if (seenValues.has(value)) return;
      seenValues.add(value);
      if (predicate(value, path)) matches.push({ value, path });
    };

    const walkFiber = (fiber, path, depth) => {
      if (!fiber || typeof fiber !== "object" || seenFibers.has(fiber) || depth > 140 || matches.length >= limit) return;
      seenFibers.add(fiber);
      const props = fiber.memoizedProps;
      inspectValue(props?.playerAPI, `${path}.memoizedProps.playerAPI`);
      inspectValue(props?.connectAPI, `${path}.memoizedProps.connectAPI`);
      inspectValue(props?.connectApi, `${path}.memoizedProps.connectApi`);
      inspectValue(props?.value, `${path}.memoizedProps.value`);
      inspectValue(props, `${path}.memoizedProps`);
      inspectValue(fiber.stateNode, `${path}.stateNode`);
      inspectValue(fiber.memoizedState, `${path}.memoizedState`);
      if (fiber.child) walkFiber(fiber.child, `${path}.child`, depth + 1);
      if (fiber.sibling) walkFiber(fiber.sibling, `${path}.sibling`, depth);
    };

    for (const [index, element] of Array.from(document.querySelectorAll("*")).entries()) {
      for (const key of Object.getOwnPropertyNames(element)) {
        if (key.startsWith("__reactContainer$") || key.startsWith("__reactFiber$")) {
          walkFiber(element[key], `element[${index}].${key}`, 0);
          if (matches.length >= limit) return matches;
        }
      }
    }

    return matches;
  };

  const findOfficialPlayerApi = () => {
    if (hasPlayerApiShape(window.__spotifyMixerOraclePlayerAPI)) return window.__spotifyMixerOraclePlayerAPI;
    const match = inspectReactFiberValues((value) => hasPlayerApiShape(value), 1)[0];
    if (!match) return null;
    window.__spotifyMixerOraclePlayerAPI = match.value;
    window.__spotifyMixerOraclePlayerAPIPath = match.path;
    return match.value;
  };

  const findOfficialConnectApi = () => {
    if (hasConnectApiShape(window.__spotifyMixerOracleConnectAPI)) return window.__spotifyMixerOracleConnectAPI;
    const match = inspectReactFiberValues((value) => hasConnectApiShape(value), 1)[0];
    if (!match) return null;
    window.__spotifyMixerOracleConnectAPI = match.value;
    window.__spotifyMixerOracleConnectAPIPath = match.path;
    return match.value;
  };

  const controlOrigin = (playerApi, featureIdentifier = CONTROL_FEATURE_ID) => ({
    featureIdentifier,
    referrerIdentifier:
      typeof playerApi?.getReferrer === "function"
        ? playerApi.getReferrer() ?? CONTROL_FEATURE_ID
        : CONTROL_FEATURE_ID,
  });

  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  const summarizeTrack = (track, options = {}) => {
    if (!track) return null;
    const fields = materializedFields(track.metadata ?? {});
    const summary = {
      uri: track.uri ?? null,
      uid: track.uid ?? null,
      name: track.name ?? track.metadata?.title ?? null,
      type: track.type ?? null,
      hasMaterializedFields: Object.keys(fields).length > 0,
      materializedKeys: Object.keys(fields).sort(),
    };
    if (options.includeFields) summary.fields = fields;
    return summary;
  };

  const summarizeContext = (context) => {
    if (!context) return null;
    const metadata = mixerContextFields(context.metadata ?? {});
    return {
      uri: context.uri ?? null,
      isMixerEnabled: isMixerContext(context),
      metadata,
    };
  };

  const officialPlayerSnapshot = () => {
    const playerApi = findOfficialPlayerApi();
    if (!playerApi) {
      return {
        available: false,
        discoveryPath: null,
      };
    }

    const state = typeof playerApi.getState === "function" ? playerApi.getState() : null;
    const queue = typeof playerApi.getQueue === "function" ? playerApi.getQueue() : null;
    const current = queue?.current ?? state?.item ?? null;
    const nextItems = Array.isArray(queue?.nextUp) ? queue.nextUp : state?.nextItems ?? [];

    return {
      available: true,
      controlPath: "official-player-api",
      discoveryPath: window.__spotifyMixerOraclePlayerAPIPath ?? null,
      methods: methodNames(playerApi).filter((name) =>
        /^(?:addToQueue|clearQueue|getCapabilities|getQueue|getReferrer|getState|insertIntoQueue|pause|play|playAsNextInQueue|refreshCurrentContext|removeFromQueue|reorderQueue|resume|seekBackward|seekBy|seekForward|seekTo|setRepeat|setShuffle|skipTo|skipToNext|skipToPrevious|updateContext)$/.test(name),
      ),
      capabilities:
        typeof playerApi.getCapabilities === "function"
          ? cloneSafe(playerApi.getCapabilities())
          : null,
      context: summarizeContext(state?.context),
      state: {
        isPaused: state?.isPaused ?? null,
        positionAsOfTimestamp: state?.positionAsOfTimestamp ?? null,
        duration: state?.duration ?? null,
        hasContext: state?.hasContext ?? null,
        item: summarizeTrack(state?.item),
        nextItems: Array.isArray(state?.nextItems) ? state.nextItems.slice(0, 10).map(summarizeTrack) : [],
      },
      queue: {
        current: summarizeTrack(current),
        queued: Array.isArray(queue?.queued) ? queue.queued.slice(0, 10).map(summarizeTrack) : [],
        nextUp: Array.isArray(nextItems) ? nextItems.slice(0, 20).map(summarizeTrack) : [],
      },
      materializedTransitionAvailable: Boolean(
        current &&
          nextItems?.[0] &&
          hasMaterializedFields(current.metadata) &&
          hasMaterializedFields(nextItems[0].metadata),
      ),
    };
  };

  const discoverControlApis = () => {
    const modules = window.__webpack_modules__ ?? {};
    return {
      officialPlayerApi: {
        available: Boolean(findOfficialPlayerApi()),
        discoveryPath: window.__spotifyMixerOraclePlayerAPIPath ?? null,
      },
      officialConnectApi: {
        available: Boolean(findOfficialConnectApi()),
        discoveryPath: window.__spotifyMixerOracleConnectAPIPath ?? null,
      },
      modules: {
        playerApiSymbolModuleId: findModule(modules, (source) => source.includes("Symbol(PlayerAPI)") || source.includes("PlayerAPI")),
        automixModuleId: findModule(modules, (source) => source.includes(SERVICE) && source.includes(METHOD)),
        esperantoTransportModuleId: findModule(modules, (source) =>
          source.includes("executeEsperantoCall") && source.includes("cancelEsperantoCall")
        ),
      },
    };
  };

  const installAutomixHook = (oracle) => {
    const modules = window.__webpack_modules__;
    if (!modules) return { installed: false, reason: "webpack modules unavailable" };
    const require = installRspackRequire();
    const transportModuleId = findModule(modules, (source) =>
      source.includes("executeEsperantoCall") && source.includes("cancelEsperantoCall")
    );
    if (transportModuleId === null) return { installed: false, reason: "Esperanto transport module not found" };

    const transportExports = require(transportModuleId);
    const Transport = transportExports.h6;
    if (!Transport?.prototype?.callSingle) return { installed: false, reason: "Esperanto Transport.callSingle not found" };

    if (!Transport.prototype.__spotifyMixerOracleOriginalCallSingle) {
      const original = Transport.prototype.callSingle;
      Object.defineProperty(Transport.prototype, "__spotifyMixerOracleOriginalCallSingle", {
        value: original,
      });
      Transport.prototype.callSingle = function (request, options) {
        if (request?.service !== SERVICE || request?.method !== METHOD) {
          return original.call(this, request, options);
        }
        const record = {
          sequence: oracle.rawRpcRecords.length,
          startedAt: new Date().toISOString(),
          service: request.service,
          method: request.method,
          requestBase64: bytesToBase64(request.payload ?? textEncoder.encode("")),
          responseBase64: null,
          elapsedMs: null,
          error: null,
        };
        oracle.rawRpcRecords.push(record);
        const started = performance.now();
        return original.call(this, request, options).then(
          (response) => {
            record.elapsedMs = performance.now() - started;
            record.responseBase64 = bytesToBase64(response);
            return response;
          },
          (error) => {
            record.elapsedMs = performance.now() - started;
            record.error = String(error);
            throw error;
          },
        );
      };
    }

    const automixModuleId = findModule(modules, (source) =>
      source.includes(SERVICE) && source.includes(METHOD)
    );
    return { installed: true, automixModuleId, transportModuleId };
  };

  const extractTrackCandidate = (value, path) => {
    if (!value || typeof value !== "object") return null;
    const metadata =
      value.metadata ??
      value.formatListAttributes ??
      value.format_list_attributes ??
      value.contextTrack?.metadata ??
      value.track?.metadata;
    if (!metadata || typeof metadata !== "object" || !hasMaterializedFields(metadata)) return null;
    return {
      path,
      uri:
        value.uri ??
        value.trackUri ??
        value.track_uri ??
        value.contextTrack?.uri ??
        value.track?.uri ??
        metadata.uri ??
        null,
      name: value.name ?? value.track?.name ?? metadata.title ?? null,
      type: value.type ?? value.track?.type ?? null,
      uid: value.uid ?? value.rowId ?? value.row_id ?? null,
      fields: materializedFields(metadata),
    };
  };

  const collectTrackCandidates = (roots) => {
    const result = [];
    const seen = new WeakSet();
    let visited = 0;
    const walk = (value, path, depth) => {
      if (visited++ > 20000 || depth > 12) return;
      if (!value || typeof value !== "object") return;
      if (seen.has(value)) return;
      seen.add(value);
      const candidate = extractTrackCandidate(value, path);
      if (candidate) result.push(candidate);
      if (Array.isArray(value)) {
        value.forEach((entry, index) => walk(entry, `${path}[${index}]`, depth + 1));
        return;
      }
      for (const [key, child] of Object.entries(value)) {
        if (typeof child !== "function") walk(child, path ? `${path}.${key}` : key, depth + 1);
      }
    };
    for (const [name, root] of Object.entries(roots)) walk(root, name, 0);
    return result;
  };

  const makeTransitions = (tracks) => {
    const transitions = [];
    const seen = new Set();
    for (let index = 0; index < tracks.length - 1; index++) {
      const outgoing = tracks[index];
      const incoming = tracks[index + 1];
      const outgoingHas = Object.keys(outgoing.fields).some((key) => key.startsWith("audio.fade_out_") || key.startsWith("automix."));
      const incomingHas = Object.keys(incoming.fields).some((key) => key.startsWith("audio.fade_in_") || key === "audio.fade_overlap" || key === "audio.speed_automation");
      if (!outgoingHas || !incomingHas) continue;
      const transition = {
        capturedAt: new Date().toISOString(),
        pair: {
          outgoingUri: outgoing.uri,
          incomingUri: incoming.uri,
          outgoingPath: outgoing.path,
          incomingPath: incoming.path,
          outgoingName: outgoing.name,
          incomingName: incoming.name,
        },
        materialized: {
          outgoing: { uri: outgoing.uri, fields: outgoing.fields },
          incoming: { uri: incoming.uri, fields: incoming.fields },
        },
      };
      const key = JSON.stringify([transition.pair.outgoingUri, transition.pair.incomingUri, transition.materialized]);
      if (seen.has(key)) continue;
      seen.add(key);
      transitions.push(transition);
    }
    return transitions;
  };

  const mediaMute = () => {
    let clickedMuteButton = false;
    for (const element of document.querySelectorAll("audio,video")) {
      element.muted = true;
      element.volume = 0;
    }
    const button = Array.from(document.querySelectorAll("button,[role='button']")).find((element) =>
      /^mute$/i.test(element.getAttribute("aria-label") ?? "") &&
      !element.disabled &&
      element.getAttribute("aria-disabled") !== "true"
    );
    if (button) {
      button.click();
      clickedMuteButton = true;
    }
    return {
      mediaElementsMuted: document.querySelectorAll("audio,video").length,
      clickedMuteButton,
      alreadyMuted: Boolean(
        Array.from(document.querySelectorAll("button,[role='button']")).find((element) =>
          /^unmute$/i.test(element.getAttribute("aria-label") ?? "")
        ),
      ),
    };
  };

  const clickElement = (selector, predicate) => {
    const element = Array.from(document.querySelectorAll(selector)).find((element) =>
      predicate({
        element,
        aria: element.getAttribute("aria-label") ?? "",
        testid: element.getAttribute("data-testid") ?? "",
        text: (element.innerText ?? element.textContent ?? "").trim(),
      }) &&
      !Boolean(element.disabled) &&
      element.getAttribute("aria-disabled") !== "true"
    );
    if (!element) return false;
    element.scrollIntoView?.({ block: "center" });
    element.click();
    return true;
  };

  const clickButton = (predicate) => clickElement("button,[role='button']", predicate);

  const requireOfficialPlayerApi = () => {
    const playerApi = findOfficialPlayerApi();
    if (!playerApi) throw new Error("No official XPUI PlayerAPI was found");
    return playerApi;
  };

  const controlResult = (action, extra = {}) => ({
    action,
    controlPath: extra.controlPath ?? "official-player-api",
    ...extra,
    officialPlayer: officialPlayerSnapshot(),
  });

  const controls = {
    status: () => ({
      hook: oracle.hook,
      hasSpicetify: Boolean(window.Spicetify),
      discovery: discoverControlApis(),
      officialPlayer: officialPlayerSnapshot(),
    }),
    mute: () => cloneSafe(controlResult("mute", { controlPath: "dom", mute: mediaMute() })),
    playUri: async (uri) => {
      mediaMute();
      const playerApi = findOfficialPlayerApi();
      if (playerApi) {
        await playerApi.play({ uri }, controlOrigin(playerApi));
        return cloneSafe(controlResult("play-uri", { uri }));
      }
      if (window.Spicetify?.Player?.playUri) return cloneSafe(await window.Spicetify.Player.playUri(uri));
      if (window.Spicetify?.Platform?.PlayerAPI?.play) {
        return cloneSafe(await window.Spicetify.Platform.PlayerAPI.play({ uri }));
      }
      throw new Error("No supported in-page playUri API was found");
    },
    playContext: async (options) => {
      const playerApi = requireOfficialPlayerApi();
      const contextUri = options?.contextUri;
      if (!contextUri) throw new Error("playContext requires contextUri");
      mediaMute();
      const playOptions = {};
      if (options.skipUri) playOptions.skipTo = { uri: options.skipUri };
      if (Number.isFinite(Number(options.skipIndex))) playOptions.skipTo = { index: Number(options.skipIndex) };
      if (Number.isFinite(Number(options.seekMs))) playOptions.seekTo = Number(options.seekMs);
      if (options.paused) playOptions.paused = true;
      await playerApi.play({ uri: contextUri }, controlOrigin(playerApi), playOptions);
      return cloneSafe(controlResult("play-context", { contextUri, playOptions }));
    },
    playPair: async (options) => {
      const playerApi = requireOfficialPlayerApi();
      const firstUri = options?.firstUri;
      const secondUri = options?.secondUri;
      if (!firstUri || !secondUri) throw new Error("playPair requires firstUri and secondUri");
      mediaMute();
      const playOptions = {};
      if (Number.isFinite(Number(options.seekMs))) playOptions.seekTo = Number(options.seekMs);
      if (options.paused) playOptions.paused = true;
      await playerApi.play({ uri: firstUri }, controlOrigin(playerApi), playOptions);
      await sleep(750);
      await playerApi.addToQueue([{ uri: secondUri, uid: null }], { interactionId: CONTROL_FEATURE_ID });
      let seekedTo = null;
      if (options.seekNearTransition) {
        await sleep(500);
        const state = playerApi.getState();
        const fadeOutStart = Number(state?.item?.metadata?.["audio.fade_out_start_time"]);
        const fallback = Number(state?.duration) - 12000;
        seekedTo = Math.max(0, Number.isFinite(fadeOutStart) ? fadeOutStart - 3000 : fallback);
        if (Number.isFinite(seekedTo)) await playerApi.seekTo(seekedTo);
      }
      return cloneSafe(controlResult("play-pair", { firstUri, secondUri, seekedTo }));
    },
    seek: async (positionMs) => {
      const numericPositionMs = Number(positionMs);
      if (!Number.isFinite(numericPositionMs) || numericPositionMs < 0) throw new Error(`invalid seek position: ${positionMs}`);
      const playerApi = findOfficialPlayerApi();
      if (playerApi) {
        await playerApi.seekTo(numericPositionMs);
        return cloneSafe(controlResult("seek", { positionMs: numericPositionMs }));
      }
      if (window.Spicetify?.Player?.seek) return cloneSafe(await window.Spicetify.Player.seek(positionMs));
      if (window.Spicetify?.Platform?.PlayerAPI?.seekTo) {
        return cloneSafe(await window.Spicetify.Platform.PlayerAPI.seekTo(numericPositionMs));
      }
      throw new Error("No supported in-page seek API was found");
    },
    next: async () => {
      const playerApi = findOfficialPlayerApi();
      if (playerApi) {
        await playerApi.skipToNext(controlOrigin(playerApi));
        return cloneSafe(controlResult("next"));
      }
      if (window.Spicetify?.Player?.next) return cloneSafe(await window.Spicetify.Player.next());
      if (window.Spicetify?.Platform?.PlayerAPI?.next) return cloneSafe(await window.Spicetify.Platform.PlayerAPI.next());
      if (clickButton(({ aria, testid }) => testid === "control-button-skip-forward" || /^next$/i.test(aria))) {
        await sleep(500);
        return cloneSafe(controlResult("next", { controlPath: "dom" }));
      }
      throw new Error("No supported in-page next API was found");
    },
    pause: async () => {
      const playerApi = findOfficialPlayerApi();
      if (playerApi) {
        const state = playerApi.getState();
        if (!state?.isPaused) await playerApi.pause(controlOrigin(playerApi));
        return cloneSafe(controlResult("pause", { alreadyPaused: Boolean(state?.isPaused) }));
      }
      if (window.Spicetify?.Player?.pause) return cloneSafe(await window.Spicetify.Player.pause());
      if (window.Spicetify?.Platform?.PlayerAPI?.pause) return cloneSafe(await window.Spicetify.Platform.PlayerAPI.pause());
      if (clickButton(({ aria, testid }) => testid === "control-button-playpause" && /^pause$/i.test(aria))) {
        await sleep(500);
        return cloneSafe(controlResult("pause", { controlPath: "dom" }));
      }
      throw new Error("No supported in-page pause API was found");
    },
    transfer: async (options) => {
      const connectApi = findOfficialConnectApi();
      if (connectApi) {
        const devices = connectApi.getDevices() ?? [];
        const target = devices.find((device) =>
          (options?.deviceId && (device.id === options.deviceId || device.connectStateId === options.deviceId)) ||
          (options?.deviceName && device.name === options.deviceName)
        );
        if (!target) throw new Error(`target Connect device was not found: ${options?.deviceName ?? options?.deviceId}`);
        if (typeof connectApi.transferPlayback === "function") {
          await connectApi.transferPlayback(target.id ?? target.connectStateId, { interactionId: CONTROL_FEATURE_ID });
        } else if (typeof connectApi.activateDevice === "function") {
          await connectApi.activateDevice(target.id ?? target.connectStateId);
        } else if (typeof connectApi.setActiveDevice === "function") {
          await connectApi.setActiveDevice(target.id ?? target.connectStateId);
        }
        return cloneSafe(controlResult("transfer", { device: summarizeContext({ uri: target.id, metadata: {} }) }));
      }

      if (!options?.deviceName) throw new Error("DOM transfer fallback requires deviceName");
      const opened = clickButton(({ aria }) => /^connect to a device$/i.test(aria));
      if (!opened) throw new Error("Connect device picker button was not found");
      await sleep(1000);
      const clicked = clickElement("li,[role='listitem'],button,[role='button']", ({ text, aria }) =>
        (text && text.includes(options.deviceName)) || (aria && aria.includes(options.deviceName))
      );
      if (!clicked) throw new Error(`Connect device was not visible in the picker: ${options.deviceName}`);
      await sleep(1000);
      return cloneSafe(controlResult("transfer", { controlPath: "dom", deviceName: options.deviceName }));
    },
  };

  const oracle = window.spotifyMixerOracle ?? {
    format: "spotify-mixer-xpui-oracle-v1",
    installedAt: new Date().toISOString(),
    rawRpcRecords: [],
    snapshots: [],
  };

  oracle.hook = installAutomixHook(oracle);
  oracle.clear = () => {
    oracle.rawRpcRecords.length = 0;
    oracle.snapshots.length = 0;
    return { cleared: true };
  };
  oracle.snapshot = () => {
    mediaMute();
    const officialPlayer = officialPlayerSnapshot();
    const playerApi = findOfficialPlayerApi();
    const roots = {
      officialPlayerQueue: playerApi?.getQueue?.(),
      spicetifyPlayerData: window.Spicetify?.Player?.data,
      spicetifyQueue: window.Spicetify?.Queue,
      spotifyPlayerData: window.__spotifyPlayer,
      mixerOracleRpcRecords: oracle.rawRpcRecords,
    };
    const tracks = collectTrackCandidates(roots);
    const transitions = makeTransitions(tracks);
    const snapshot = {
      format: oracle.format,
      capturedAt: new Date().toISOString(),
      href: location.href,
      hook: oracle.hook,
      officialPlayer,
      player: cloneSafe({
        item: window.Spicetify?.Player?.data?.item,
        context: window.Spicetify?.Player?.data?.context,
        nextItems: window.Spicetify?.Player?.data?.nextItems,
      }),
      tracks,
      transitions,
      rpcRecords: cloneSafe(oracle.rawRpcRecords),
      summary: {
        tracksWithMaterializedFields: tracks.length,
        transitions: transitions.length,
        rpcRecords: oracle.rawRpcRecords.length,
      },
    };
    oracle.snapshots.push(snapshot);
    return snapshot;
  };
  oracle.exportJson = () => JSON.stringify({
    format: oracle.format,
    installedAt: oracle.installedAt,
    hook: oracle.hook,
    snapshots: oracle.snapshots,
    rawRpcRecords: cloneSafe(oracle.rawRpcRecords),
  }, null, 2);
  oracle.controls = controls;
  window.spotifyMixerOracle = oracle;

  return {
    installed: true,
    format: oracle.format,
    hook: oracle.hook,
    hasSpicetify: Boolean(window.Spicetify),
    control: discoverControlApis(),
  };
})();
