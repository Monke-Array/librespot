(() => {
  const SERVICE = "spotify.automix.esperanto.proto.Automix";

  const modules = window.__webpack_modules__;
  const chunkName = Object.keys(window).find(k => k.startsWith("rspackChunk"));
  if (!modules || !chunkName) throw new Error("Rspack runtime unavailable");

  if (!window.__spotifyRspackRequire) {
    window[chunkName].push([[987654322], {}, require => {
      window.__spotifyRspackRequire = require;
    }]);
  }

  const require = window.__spotifyRspackRequire;

  const findModule = predicate => {
    const match = Object.entries(modules)
      .find(([, factory]) => predicate(String(factory)));

    if (!match) throw new Error("Required module not found");
    return Number(match[0]);
  };

  const automixModuleId = findModule(src =>
    src.includes(SERVICE) &&
    src.includes("GetComputedTransitions") &&
    src.includes("GetStylesForPresetId")
  );

  const transportModuleId = findModule(src =>
    src.includes("executeEsperantoCall") &&
    src.includes("cancelEsperantoCall")
  );

  const Automix = require(automixModuleId).N5;
  const transportExports = require(transportModuleId);
  const Transport = transportExports.h6;

  const bytesToBase64 = value => {
    const bytes = value instanceof Uint8Array
      ? value
      : new Uint8Array(value);

    let binary = "";
    for (let i = 0; i < bytes.length; i += 32768) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
    }
    return btoa(binary);
  };

  const normalize = value => {
    if (value === null || value === undefined)
      return value;

    if (typeof value === "bigint")
      return value.toString();

    if (value instanceof Uint8Array)
      return {
        __type: "bytes",
        base64: bytesToBase64(value),
      };

    if (Array.isArray(value))
      return value.map(normalize);

    if (typeof value === "object") {
      const out = {};
      for (const [k, v] of Object.entries(value))
        out[k] = normalize(v);
      return out;
    }

    return value;
  };

  const records = window.spotifyAutomixOracleAll?.records ?? [];

  const baseOriginal =
    Transport.prototype.__spotifyAutomixOracleOriginalCallSingle ??
    Transport.prototype.callSingle;

  if (!Transport.prototype.__spotifyAutomixAllInstalled) {
    Object.defineProperty(
      Transport.prototype,
      "__spotifyAutomixAllInstalled",
      { value: true }
    );

    Transport.prototype.callSingle = function(request, options) {
      if (request?.service !== SERVICE)
        return baseOriginal.call(this, request, options);

      const decoder = Automix.DECODERS?.[request.method];

      const record = {
        sequence: records.length,
        startedAt: new Date().toISOString(),
        method: request.method,
        request: null,
        response: null,
        requestBase64: bytesToBase64(request.payload),
        responseBase64: null,
        elapsedMs: null,
        error: null,
      };

      try {
        if (decoder?.request) {
          record.request = normalize(
            decoder.request(request.payload)
          );
        }
      } catch (e) {
        record.requestDecodeError = String(e);
      }

      records.push(record);

      const started = performance.now();

      return baseOriginal.call(this, request, options).then(
        response => {
          record.elapsedMs = performance.now() - started;
          record.responseBase64 = bytesToBase64(response);

          try {
            if (decoder?.response) {
              record.response = normalize(
                decoder.response(response)
              );
            }
          } catch (e) {
            record.responseDecodeError = String(e);
          }

          return response;
        },
        error => {
          record.elapsedMs = performance.now() - started;
          record.error = String(error);
          throw error;
        }
      );
    };
  }

  window.spotifyAutomixOracleAll = {
    automixModuleId,
    transportModuleId,
    records,

    clear() {
      records.length = 0;
    },

    summary() {
      return records.reduce((acc, r) => {
        acc[r.method] = (acc[r.method] ?? 0) + 1;
        return acc;
      }, {});
    },

    exportJson() {
      return JSON.stringify({
        format: "spotify-automix-runtime-oracle-v1",
        capturedAt: new Date().toISOString(),
        automixModuleId,
        transportModuleId,
        records,
      }, null, 2);
    }
  };

  return {
    installed: true,
    automixModuleId,
    transportModuleId,
    existingRecords: records.length,
    methods: Object.keys(Automix.DECODERS ?? {})
  };
})();
