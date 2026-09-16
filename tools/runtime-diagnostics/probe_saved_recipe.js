"use strict";

// Read-only saved-recipe probe using the signed-in official client's existing
// Esperanto metadata transport. No token/cookie export, playback, or playlist writes.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { evaluateWithPowerShell } = require("../spotify-mixer-harness/lib/cdp");

function probeExpression(transitionUri) {
  if (!/^spotify:transition:[A-Za-z0-9]+:[0-9]+$/.test(transitionUri)) {
    throw new Error("Expected an explicitly selected saved transition URI with revision");
  }
  return `(${async function (uri) {
    const modules = window.__webpack_modules__;
    const require = window.__spotifyRspackRequire;
    if (!modules || !require) throw new Error("Install the existing mixer oracle first");
    const find = (predicate) => {
      const matches = Object.entries(modules).filter(([, fn]) => predicate(String(fn)));
      if (matches.length !== 1) throw new Error(`Expected one codec module, got ${matches.length}`);
      return matches[0][0];
    };
    const metadataId = find(s => s.includes('SERVICE_ID="spotify.mdata_esperanto.proto.MetadataService"'));
    const transportId = find(s => s.includes("executeEsperantoCall") && s.includes("cancelEsperantoCall"));
    const decoderId = find(s => s.includes("latestTransitionUri") && s.includes("creatorUserId"));
    const Metadata = require(metadataId).sp;
    const decode = require(decoderId).iy;
    if (typeof Metadata !== "function" || typeof decode !== "function") {
      throw new Error("Installed codec exports changed; inspect before continuing");
    }
    const requestStartedAt = new Date().toISOString();
    const response = await new Metadata(require(transportId).n1()).fetch({
      extensionQuery: [{ extensionKind: 244, entityUri: [uri] }],
    });
    const responseReceivedAt = new Date().toISOString();
    const group = response.extension?.find(x => x.extensionKind === 244);
    const entity = group?.entityExtension?.find(x => x.entityUri === uri);
    if (entity?.header?.statusCode !== 200 || !entity.extensionData?.value) {
      throw new Error(`Saved transition unavailable: ${entity?.header?.statusCode ?? "missing"}`);
    }
    const data = decode(244, entity.extensionData.value);
    if (data?.transitionUri !== uri || !data.transition) throw new Error("Mismatched or empty TransitionData");
    // Explicit allowlist excludes creatorUserId and the original enclosing payload.
    return {
      requestStartedAt, responseReceivedAt,
      source: "explicit diagnostic MetadataService.Fetch; cache/network origin not distinguished",
      service: Metadata.SERVICE_ID, method: "Fetch", extensionKind: 244,
      typeUrl: entity.extensionData.typeUrl, status: entity.header.statusCode,
      transitionUri: data.transitionUri, latestTransitionUri: data.latestTransitionUri,
      playlistUri: data.playlistUri, trackAUri: data.trackAUri, trackBUri: data.trackBUri,
      recipeBase64: data.transition,
      codecModules: { metadataId, transportId, decoderId },
    };
  }.toString()})(${JSON.stringify(transitionUri)})`;
}

function main() {
  const [uri, output] = process.argv.slice(2);
  if (!uri || !output) throw new Error("Usage: node probe_saved_recipe.js <transition-uri> <output.json>");
  const result = evaluateWithPowerShell({
    repoRoot: path.resolve(__dirname, "../.."), expression: probeExpression(uri), port: 9222,
  });
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(result.recipeBase64) || result.recipeBase64.length > 16384) {
    throw new Error("Invalid or oversized recipe payload");
  }
  const bytes = Buffer.from(result.recipeBase64, "base64");
  result.recipeSha256 = crypto.createHash("sha256").update(bytes).digest("hex");
  result.recipeBytes = bytes.length;
  fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(result, null, 2) + "\n", { flag: "wx" });
  console.log(JSON.stringify({ output, status: result.status, recipeSha256: result.recipeSha256, recipeBytes: result.recipeBytes }));
}

if (require.main === module) main();
module.exports = { probeExpression };
