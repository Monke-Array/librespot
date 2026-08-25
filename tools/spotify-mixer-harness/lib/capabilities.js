"use strict";

const ANSI_PATTERN = /\x1b\[[0-9;]*m/g;

const CAPABILITY_BOOLEAN_FIELDS = {
  "can-be-player": "canBePlayer",
  "restrict-to-local": "restrictToLocal",
  "gaia-eq-connect-id": "gaiaEqConnectId",
  "supports-logout": "supportsLogout",
  observable: "observable",
  "command-acks": "commandAcks",
  "supports-rename": "supportsRename",
  hidden: "hidden",
  "disable-volume": "disableVolume",
  "connect-disabled": "connectDisabled",
  "supports-playlist-v2": "supportsPlaylistV2",
  controllable: "controllable",
  "supports-external-episodes": "supportsExternalEpisodes",
  "supports-set-backend-metadata": "supportsSetBackendMetadata",
  "supports-transfer-command": "supportsTransferCommand",
  "supports-command-request": "supportsCommandRequest",
  "voice-enabled": "voiceEnabled",
  "needs-full-player-state": "needsFullPlayerState",
  "supports-gzip-pushes": "supportsGzipPushes",
  "supports-set-options-command": "supportsSetOptionsCommand",
  "supports-rooms": "supportsRooms",
  "supports-dj": "supportsDj",
};

const RELEVANCE = new Map([
  ["known.capabilities.supportsDj", ["DJ", "Mixer", "transition materialization"]],
  ["known.capabilities.supportsSetBackendMetadata", ["backend metadata", "transition materialization"]],
  ["known.capabilities.needsFullPlayerState", ["full player state", "transition materialization"]],
  ["known.capabilities.supportsPlaylistMixing", ["Mixer"]],
  ["known.capabilities.supportedMediaTypes", ["media type support"]],
  ["known.capabilities.supportsExternalEpisodes", ["media type support"]],
  ["known.capabilities.supportedAudioQuality", ["media type support"]],
  ["known.capabilities.supportsHifi", ["media type support"]],
  ["unknown.capabilities", ["unknown/raw capability fields", "transition materialization"]],
  ["unknown.deviceInfo", ["unknown/raw capability fields"]],
]);

function stripAnsi(value) {
  return String(value ?? "").replace(ANSI_PATTERN, "");
}

function parseBool(value) {
  if (value === "true") return true;
  if (value === "false") return false;
  return value;
}

function parseJsonArray(value) {
  try {
    return JSON.parse(value);
  } catch {
    return [];
  }
}

function canonicalSource(source) {
  return source.trim();
}

function ensureSnapshot(snapshots, source) {
  const key = canonicalSource(source);
  if (!snapshots.has(key)) {
    snapshots.set(key, {
      source: key,
      known: {
        deviceInfo: {},
        sensitivePresence: {},
        capabilities: {},
      },
      unknown: {
        deviceInfo: [],
        capabilities: [],
        supportsHifi: [],
      },
    });
  }
  return snapshots.get(key);
}

function parseDeviceInfo(snapshot, body) {
  const match = body.match(
    /^active=(.*?) can-play=(\S+) type=(.*?) software-version=(\S+) spirc-version=(\S+) client-id=(\S+) product-id=(\S+) brand=(.*?) model=(.*?) group=(\S+) dynamic=(\S+) social-connect=(\S+) private-session=(\S+) offline=(\S+)$/,
  );
  if (!match) return;
  snapshot.known.deviceInfo = {
    active: match[1],
    canPlay: parseBool(match[2]),
    deviceType: match[3],
    softwareVersion: match[4],
    spircVersion: match[5],
    clientId: match[6],
    productId: match[7],
    brand: match[8],
    model: match[9],
    isGroup: parseBool(match[10]),
    isDynamic: parseBool(match[11]),
    isSocialConnect: parseBool(match[12]),
    isPrivateSession: parseBool(match[13]),
    isOffline: parseBool(match[14]),
  };
}

function parseSensitivePresence(snapshot, body) {
  const metadata = body.match(/metadata-keys=(\[[^\]]*\])/);
  const disallowPlayback = body.match(/disallow-playback=(\[[^\]]*\])/);
  const disallowTransfer = body.match(/disallow-transfer=(\[[^\]]*\])/);
  for (const [wire, canonical] of [
    ["name-present", "namePresent"],
    ["device-id-present", "deviceIdPresent"],
    ["deduplication-id-present", "deduplicationIdPresent"],
    ["public-ip-present", "publicIpPresent"],
    ["license-present", "licensePresent"],
  ]) {
    const match = body.match(new RegExp(`${wire}=(\\S+)`));
    if (match) snapshot.known.sensitivePresence[canonical] = parseBool(match[1]);
  }
  if (metadata) snapshot.known.sensitivePresence.metadataKeys = parseJsonArray(metadata[1]);
  if (disallowPlayback) snapshot.known.sensitivePresence.disallowPlayback = parseJsonArray(disallowPlayback[1]);
  if (disallowTransfer) snapshot.known.sensitivePresence.disallowTransfer = parseJsonArray(disallowTransfer[1]);
}

function parseCapabilityBooleans(snapshot, body) {
  for (const [wire, canonical] of Object.entries(CAPABILITY_BOOLEAN_FIELDS)) {
    const match = body.match(new RegExp(`${wire}=(true|false)`));
    if (match) snapshot.known.capabilities[canonical] = parseBool(match[1]);
  }
}

function parseCapabilityValues(snapshot, body) {
  const volumeSteps = body.match(/volume-steps=(\d+)/);
  const mediaTypes = body.match(/supported-media-types=(\[[^\]]*\])/);
  const audioQuality = body.match(/supported-audio-quality=([^\s]+)/);
  const connectCapabilities = body.match(/connect-capabilities=(\S+)/);
  if (volumeSteps) snapshot.known.capabilities.volumeSteps = Number(volumeSteps[1]);
  if (mediaTypes) snapshot.known.capabilities.supportedMediaTypes = parseJsonArray(mediaTypes[1]);
  if (audioQuality) snapshot.known.capabilities.supportedAudioQuality = audioQuality[1];
  if (connectCapabilities) snapshot.known.capabilities.connectCapabilities = connectCapabilities[1];
}

function parseSupportsHifi(snapshot, body) {
  if (body.includes("present=false")) {
    snapshot.known.capabilities.supportsHifi = { present: false };
    return;
  }
  const fields = { present: true };
  for (const [wire, canonical] of [
    ["fully-supported", "fullySupported"],
    ["user-eligible", "userEligible"],
    ["device-supported", "deviceSupported"],
  ]) {
    const match = body.match(new RegExp(`${wire}=(true|false)`));
    if (match) fields[canonical] = parseBool(match[1]);
  }
  snapshot.known.capabilities.supportsHifi = fields;
}

function parseUnknownArray(line) {
  const match = line.match(/unknown-protobuf-fields=(\[[^\]]*\])/);
  return match ? parseJsonArray(match[1]) : [];
}

function parseCapabilitySnapshots(logText) {
  const snapshots = new Map();
  let currentSource = null;
  for (const rawLine of stripAnsi(logText).split(/\r?\n/)) {
    const marker = "[spotify-capability-debug] ";
    const markerIndex = rawLine.indexOf(marker);
    if (markerIndex < 0) continue;
    const line = rawLine.slice(markerIndex + marker.length);

    let match = line.match(/^(.*?) device-info active=(.*)$/);
    if (match) {
      currentSource = canonicalSource(match[1]);
      parseDeviceInfo(ensureSnapshot(snapshots, currentSource), `active=${match[2]}`);
      continue;
    }

    match = line.match(/^(.*?) device-info sensitive-or-user-fields (.*)$/);
    if (match) {
      currentSource = canonicalSource(match[1]);
      parseSensitivePresence(ensureSnapshot(snapshots, currentSource), match[2]);
      continue;
    }

    match = line.match(/^(.*?) capability-booleans (.*)$/);
    if (match) {
      currentSource = canonicalSource(match[1]);
      parseCapabilityBooleans(ensureSnapshot(snapshots, currentSource), match[2]);
      continue;
    }

    match = line.match(/^(.*?) capability-values (.*)$/);
    if (match) {
      currentSource = canonicalSource(match[1]);
      parseCapabilityValues(ensureSnapshot(snapshots, currentSource), match[2]);
      continue;
    }

    match = line.match(/^(.*?) supports-hifi (.*)$/);
    if (match) {
      currentSource = canonicalSource(match[1]);
      parseSupportsHifi(ensureSnapshot(snapshots, currentSource), match[2]);
      continue;
    }

    if (line.startsWith("device-info unknown-protobuf-fields=") && currentSource) {
      ensureSnapshot(snapshots, currentSource).unknown.deviceInfo = parseUnknownArray(line);
      continue;
    }

    if (line.startsWith("capabilities unknown-protobuf-fields=") && currentSource) {
      ensureSnapshot(snapshots, currentSource).unknown.capabilities = parseUnknownArray(line);
      continue;
    }

    if (line.startsWith("supports-hifi unknown-protobuf-fields=") && currentSource) {
      ensureSnapshot(snapshots, currentSource).unknown.supportsHifi = parseUnknownArray(line);
    }
  }
  return [...snapshots.values()];
}

function findOfficialDesktop(snapshots, requestedSource) {
  if (requestedSource) return snapshots.find((snapshot) => snapshot.source === requestedSource);
  return (
    snapshots.find((snapshot) => {
      const device = snapshot.known.deviceInfo;
      return device.brand === "spotify" && /PC desktop/i.test(device.model ?? "");
    }) ??
    snapshots.find((snapshot) => snapshot.known.capabilities.supportsDj === true) ??
    snapshots.find((snapshot) => snapshot.source.startsWith("incoming "))
  );
}

function findSpotifyd(snapshots, requestedSource) {
  if (requestedSource) return snapshots.find((snapshot) => snapshot.source === requestedSource);
  return snapshots.find((snapshot) => snapshot.source === "outgoing");
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, stable(item)]));
  }
  return value;
}

function valueAt(object, path) {
  return path.split(".").reduce((current, part) => (current === undefined || current === null ? undefined : current[part]), object);
}

function collectPaths(object, prefix = "") {
  if (!object || typeof object !== "object" || Array.isArray(object)) return [prefix];
  return Object.keys(object).flatMap((key) => collectPaths(object[key], prefix ? `${prefix}.${key}` : key));
}

function valuesEqual(left, right) {
  return JSON.stringify(stable(left)) === JSON.stringify(stable(right));
}

function buildDifferences(official, spotifyd) {
  const paths = new Set([...collectPaths(official), ...collectPaths(spotifyd)].filter(Boolean));
  return [...paths]
    .sort()
    .filter((path) => !valuesEqual(valueAt(official, path), valueAt(spotifyd, path)))
    .map((path) => ({
      path,
      official: stable(valueAt(official, path)),
      spotifyd: stable(valueAt(spotifyd, path)),
      relevance: RELEVANCE.get(path) ?? [],
    }));
}

function buildCapabilityDiffReport({ logText, generatedAt = new Date().toISOString(), officialSource, spotifydSource }) {
  const snapshots = parseCapabilitySnapshots(logText);
  const official = findOfficialDesktop(snapshots, officialSource);
  const spotifyd = findSpotifyd(snapshots, spotifydSource);
  if (!official) throw new Error("official Spotify Desktop capability advertisement was not found in log");
  if (!spotifyd) throw new Error("spotifyd outgoing capability advertisement was not found in log");

  return {
    format: "spotify-mixer-capability-diff-v1",
    generatedAt,
    official: stable(official),
    spotifyd: stable(spotifyd),
    differences: buildDifferences(official, spotifyd),
  };
}

function summarizeTrack(item) {
  const materializedKeys = Array.isArray(item?.materializedKeys) ? item.materializedKeys : [];
  return {
    uri: item?.uri ?? null,
    name: item?.name ?? null,
    materializedKeys,
    hasAudio: materializedKeys.some((key) => key.startsWith("audio.")),
    hasAutoPresetId: materializedKeys.includes("automix.auto_preset_id"),
    hasTransitionUri: materializedKeys.includes("automix.transition_uri"),
  };
}

function summarizeMixerEndpointStatus(status) {
  const player = status?.officialPlayer ?? {};
  const context = player.context ?? {};
  const state = player.state ?? {};
  const contextMetadata = context.metadata ?? {};
  return {
    contextUri: context.uri ?? null,
    isMixerEnabled: Boolean(context.isMixerEnabled),
    contextMetadata: {
      mix: contextMetadata.mix ?? null,
      automixMode: contextMetadata["automix.mode"] ?? null,
      automixQueue: contextMetadata["automix.queue"] ?? null,
      canViewTransition: contextMetadata["can-view-transition"] ?? null,
    },
    isPaused: state.isPaused ?? null,
    positionAsOfTimestamp: state.positionAsOfTimestamp ?? null,
    materializedTransitionAvailable: Boolean(player.materializedTransitionAvailable),
    current: summarizeTrack(state.item),
    next: summarizeTrack(Array.isArray(state.nextItems) ? state.nextItems[0] : null),
  };
}

module.exports = {
  buildCapabilityDiffReport,
  parseCapabilitySnapshots,
  summarizeMixerEndpointStatus,
};
