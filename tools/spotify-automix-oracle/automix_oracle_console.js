// Paste into Spotify Desktop DevTools. This records only Automix/GetComputedTransitions.
(() => {
  const SERVICE = "spotify.automix.esperanto.proto.Automix";
  const METHOD = "GetComputedTransitions";
  const modules = window.__webpack_modules__;
  const chunkName = Object.keys(window).find((key) => key.startsWith("rspackChunk"));
  if (!modules || !chunkName) throw new Error("Rspack runtime is unavailable");

  if (!window.__spotifyRspackRequire) {
    window[chunkName].push([[987654321], {}, (require) => {
      window.__spotifyRspackRequire = require;
    }]);
  }

  const require = window.__spotifyRspackRequire;
  const findModule = (predicate) => {
    const match = Object.entries(modules).find(([, factory]) => predicate(String(factory)));
    if (!match) throw new Error("Required Spotify module was not found");
    return Number(match[0]);
  };
  const automixModuleId = findModule((source) =>
    source.includes(SERVICE) && source.includes(METHOD)
  );
  const transportModuleId = findModule((source) =>
    source.includes("executeEsperantoCall") && source.includes("cancelEsperantoCall")
  );

  const Automix = require(automixModuleId).N5;
  const transportExports = require(transportModuleId);
  const Transport = transportExports.h6;
  const makeTransport = transportExports.n1;
  const base64 = (value) => {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += 32768) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
    }
    return btoa(binary);
  };

  if (!Transport.prototype.__spotifyAutomixOracleOriginalCallSingle) {
    const original = Transport.prototype.callSingle;
    Object.defineProperty(Transport.prototype, "__spotifyAutomixOracleOriginalCallSingle", {
      value: original,
    });
    Transport.prototype.callSingle = function (request, options) {
      if (request?.service !== SERVICE || request?.method !== METHOD) {
        return original.call(this, request, options);
      }
      const record = {
        sequence: window.spotifyAutomixOracle.rawRpcRecords.length,
        startedAt: new Date().toISOString(),
        requestBase64: base64(request.payload),
        responseBase64: null,
        elapsedMs: null,
        error: null,
      };
      window.spotifyAutomixOracle.rawRpcRecords.push(record);
      const started = performance.now();
      return original.call(this, request, options).then(
        (response) => {
          record.elapsedMs = performance.now() - started;
          record.responseBase64 = base64(response);
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

  const client = new Automix(makeTransport());
  window.spotifyAutomixOracle = {
    service: SERVICE,
    method: METHOD,
    automixModuleId,
    transportModuleId,
    rawRpcRecords: window.spotifyAutomixOracle?.rawRpcRecords ?? [],
    decodedRuns: window.spotifyAutomixOracle?.decodedRuns ?? [],
    async capture(pairs) {
      const request = {
        trackPairs: pairs.map(({ label, ...pair }) => pair),
      };
      const response = await client.getComputedTransitions(request);
      const run = { capturedAt: new Date().toISOString(), pairs, response };
      this.decodedRuns.push(run);
      return run;
    },
    exportJson() {
      return JSON.stringify({
        format: "spotify-automix-get-computed-transitions-oracle-v1",
        source: { automixModuleId, transportModuleId },
        rpcRecords: this.rawRpcRecords,
        runs: this.decodedRuns,
      }, null, 2);
    },
  };

  return {
    installed: true,
    automixModuleId,
    transportModuleId,
  };
})();
