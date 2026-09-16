"use strict";
// Read-only native Automix RPCs. No player, playlist, authentication or audio calls.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { evaluateWithPowerShell } = require("../spotify-mixer-harness/lib/cdp");

async function captureInPage() {
  const require = window.__spotifyRspackRequire;
  if (!require) throw new Error("Install the existing mixer oracle first");
  const find = predicate => {
    const found = Object.entries(window.__webpack_modules__).filter(([, f]) => predicate(String(f)));
    if (found.length !== 1) throw new Error(`Expected one generated module, got ${found.length}`);
    return found[0][0];
  };
  const automixId = find(s => s.includes('SERVICE_ID="spotify.automix.esperanto.proto.Automix"'));
  const transportId = find(s => s.includes("executeEsperantoCall") && s.includes("cancelEsperantoCall"));
  const A = require(automixId).N5;
  const transport = require(transportId).n1();
  const a = new A(transport);
  const bytes64 = bytes => {
    let text = "";
    for (let i = 0; i < bytes.length; i += 32768) text += String.fromCharCode(...bytes.subarray(i, i + 32768));
    return btoa(text);
  };
  const records = [];
  const call = async (method, request) => {
    const before = new Date().toISOString();
    try {
      const response = await a[method](request);
      records.push({ method, request, before, after: new Date().toISOString(), response });
      return response;
    } catch (error) {
      records.push({ method, request, before, error: String(error).slice(0,300) });
      return null;
    }
  };
  const presets = await call("getStylesForPresetId", { presetIds: Array.from({ length: 21 }, (_, i) => i) });
  await call("getStylesForPresetId", { presetIds: [-1, 999] });
  const kinds = [
    ["volumeStyle", "getVolumeStylesForVolumeStyleId", "volumeStyleIds"],
    ["eqStyle", "getEqStylesForEqStyleId", "eqStyleIds"],
    ["filterFxStyle", "getFilterFxStylesForFilterFxStyleId", "filterFxStyleIds"],
    ["fxStyle", "getFxStylesForFxStyleId", "fxStyleIds"],
  ];
  for (const [field, method, requestKey] of kinds) {
    const ids = [...new Set([0, ...(presets?.presetStyles ?? []).map(p => p[field]?.id).filter(x => x !== undefined)])].sort((a,b)=>a-b);
    for (const numBars of [0, 1, 2, 3, 4, 8, 16, 32]) {
      await call(method, { [requestKey]: ids, numBars });
    }
    await call(method, { [requestKey]: [999], numBars: 4 });
  }
  await call("getMaxTransitionLength", {});
  await call("getBeatmatchOptOutTransitionDuration", {});
  let bundle;
  try {
    const payload = await transport.callSingle({
      service: "spotify.automix.bundle.esperanto.proto.AutomixBundle",
      method: "GetMixingBundle", payload: new Uint8Array(),
    });
    bundle = { service: "spotify.automix.bundle.esperanto.proto.AutomixBundle", method: "GetMixingBundle", responseBase64: bytes64(payload) };
  } catch (error) {
    bundle = { error: String(error).slice(0,300) };
  }
  return { format: "spotify-automix-style-probe-v1", capturedAt: new Date().toISOString(), automixId, transportId, bundle, records };
}

function main() {
  const output = process.argv[2];
  if (!output) throw new Error("Usage: node probe_automix_styles.js <private-output.json>");
  const result = evaluateWithPowerShell({ repoRoot: path.resolve(__dirname, "../.."), expression: `(${captureInPage.toString()})()` });
  if (result.bundle?.responseBase64) {
    result.bundle.responseSha256 = crypto.createHash("sha256").update(Buffer.from(result.bundle.responseBase64,"base64")).digest("hex");
  }
  fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(result,null,2)+"\n", { flag: "wx" });
  console.log(JSON.stringify({ output, calls: result.records.length, errors: result.records.filter(r=>r.error).map(r=>({method:r.method,request:r.request,error:r.error})), bundle: result.bundle.responseSha256 ?? result.bundle.error }));
}
if (require.main === module) main();
module.exports = { captureInPage };
