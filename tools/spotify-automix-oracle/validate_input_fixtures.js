// Validates the captured native inputs against the official output oracle.
// This is deliberately analysis-only; it is not the future Rust producer.
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const root = __dirname;
const oracle = JSON.parse(fs.readFileSync(
  path.join(root, "get_computed_transitions_2026-08-14.json"),
  "utf8",
));
const run = oracle.runs[0];
const trackCache = new Map();

function track(uri) {
  const id = uri.split(":").at(-1);
  if (!trackCache.has(id)) {
    trackCache.set(id, JSON.parse(fs.readFileSync(path.join(root, "tracks", `${id}.json`), "utf8")));
  }
  return trackCache.get(id);
}

function downbeatIndexByMs(fixture) {
  return new Map(fixture.decoded.normalizedDownbeats.map((point, index) => [point.positionMs, index]));
}

function putI32Le(bytes, value) {
  const buffer = Buffer.alloc(4);
  buffer.writeInt32LE(value);
  bytes.push(buffer);
}

function expectedPreset(overlap) {
  if (!overlap.isBeatmatched) return 1;
  const pool = overlap.durationBars < 4
    ? [1, 10, 19]
    : [1, 2, 3, 4, 5, 17, 18, 8, 9, 10, 19];
  const bytes = [
    Buffer.from(overlap.trackAPlayableUri, "utf8"),
    Buffer.from(overlap.trackBPlayableUri, "utf8"),
  ];
  putI32Le(bytes, overlap.durationBars);
  putI32Le(bytes, overlap.startAMs);
  putI32Le(bytes, overlap.startBMs);
  const digest = crypto.createHash("sha1").update(Buffer.concat(bytes)).digest();
  const selector = digest.readBigUInt64LE(0) % BigInt(pool.length);
  return pool[Number(selector)];
}

const perPair = [];
const discrepancies = [];
let total = 0;
let beatmatched = 0;
let exactGeometry = 0;
let exactSpeedBits = 0;
let exactPreset = 0;
let maximumSpeedDifference = 0;
const speedFormulaStats = new Map();

function beatIntervalRepresentations(fixture, start, end) {
  const beats = fixture.decoded.beats.beats;
  const intervals = [];
  for (let index = start.rawBeatIndex; index < end.rawBeatIndex; index++) {
    intervals.push(beats[index + 1].timeSeconds - beats[index].timeSeconds);
  }
  let float32Sum = 0;
  let float32SumOfFloat32Intervals = 0;
  let doubleSum = 0;
  let float32ReciprocalSum = 0;
  for (const interval of intervals) {
    float32Sum = Math.fround(float32Sum + interval);
    float32SumOfFloat32Intervals = Math.fround(float32SumOfFloat32Intervals + Math.fround(interval));
    doubleSum += interval;
    float32ReciprocalSum = Math.fround(float32ReciprocalSum + Math.fround(1 / interval));
  }
  return {
    intervalFloat32Sum: float32Sum,
    intervalFloat32SumOfFloat32: float32SumOfFloat32Intervals,
    intervalDoubleSum: doubleSum,
    intervalFloat32Mean: Math.fround(float32Sum / intervals.length),
    reciprocalFloat32Mean: Math.fround(float32ReciprocalSum / intervals.length),
  };
}

function evaluateSpeedFormulas(fixtureA, startA, endA, fixtureB, startB, endB, actual) {
  const intervalsA = beatIntervalRepresentations(fixtureA, startA, endA);
  const intervalsB = beatIntervalRepresentations(fixtureB, startB, endB);
  const representations = {
    rawDouble: (end, start) => end.positionSeconds - start.positionSeconds,
    rawFloat32: (end, start) => Math.fround(end.positionSeconds - start.positionSeconds),
    outputMillisecondsDouble: (end, start) => (end.positionMs - start.positionMs) * 0.001,
    outputMillisecondsFloat32: (end, start) => Math.fround((end.positionMs - start.positionMs) * 0.001),
  };
  const operations = {
    divide: (durationB, durationA) => Math.fround(durationB / durationA),
    reciprocalMultiply: (durationB, durationA) => Math.fround(Math.fround(1 / durationA) * durationB),
    inverseDivide: (durationB, durationA) => Math.fround(1 / Math.fround(durationA / durationB)),
  };
  for (const [nameA, representationA] of Object.entries(representations)) {
    for (const [nameB, representationB] of Object.entries(representations)) {
      const durationA = representationA(endA, startA);
      const durationB = representationB(endB, startB);
      for (const [operationName, operation] of Object.entries(operations)) {
        const name = `${nameB}/${nameA}:${operationName}`;
        const expected = operation(durationB, durationA);
        const difference = Math.abs(expected - actual);
        const stat = speedFormulaStats.get(name) ?? { name, exact: 0, maximumDifference: 0 };
        if (Object.is(expected, actual)) stat.exact++;
        stat.maximumDifference = Math.max(stat.maximumDifference, difference);
        speedFormulaStats.set(name, stat);
      }
    }
  }
  for (const name of Object.keys(intervalsA)) {
    const durationA = intervalsA[name];
    const durationB = intervalsB[name];
    for (const [operationName, operation] of Object.entries(operations)) {
      const formulaName = `${name}:${operationName}`;
      // reciprocal mean represents frequency; its A/B ratio has the same
      // direction as durationB/durationA.
      const expected = name === "reciprocalFloat32Mean"
        ? operation(durationA, durationB)
        : operation(durationB, durationA);
      const difference = Math.abs(expected - actual);
      const stat = speedFormulaStats.get(formulaName) ?? {
        name: formulaName,
        exact: 0,
        maximumDifference: 0,
      };
      if (Object.is(expected, actual)) stat.exact++;
      stat.maximumDifference = Math.max(stat.maximumDifference, difference);
      speedFormulaStats.set(formulaName, stat);
    }
  }
}

run.pairs.forEach((pair, pairIndex) => {
  const trackA = track(pair.trackAUri);
  const trackB = track(pair.trackBUri);
  const pointsA = trackA.decoded.normalizedDownbeats;
  const pointsB = trackB.decoded.normalizedDownbeats;
  const indexesA = downbeatIndexByMs(trackA);
  const indexesB = downbeatIndexByMs(trackB);
  const pairSummary = {
    pairIndex,
    label: pair.label,
    transitions: 0,
    beatmatched: 0,
    exactGeometry: 0,
    exactSpeedBits: 0,
    exactPreset: 0,
  };

  run.response.computedTransitions[pairIndex].rankedTransitions.forEach((ranked, rank) => {
    total++;
    pairSummary.transitions++;
    const overlap = ranked.overlap;
    const actualPreset = ranked.rankedPresets[0]?.preset?.id;
    const preset = expectedPreset(overlap);
    if (preset === actualPreset) {
      exactPreset++;
      pairSummary.exactPreset++;
    } else {
      discrepancies.push({ pair: pair.label, rank, field: "preset", expected: preset, actual: actualPreset });
    }

    if (!overlap.isBeatmatched) return;
    beatmatched++;
    pairSummary.beatmatched++;
    const indexA = indexesA.get(overlap.startAMs);
    const indexB = indexesB.get(overlap.startBMs);
    const endA = pointsA[indexA + overlap.durationBars];
    const endB = pointsB[indexB + overlap.durationBars];
    const geometryMatches = indexA !== undefined
      && indexB !== undefined
      && endA !== undefined
      && endB !== undefined
      && endA.positionMs - overlap.startAMs === overlap.durationMs;
    if (geometryMatches) {
      exactGeometry++;
      pairSummary.exactGeometry++;
    } else {
      discrepancies.push({
        pair: pair.label,
        rank,
        field: "geometry",
        overlap,
        indexA,
        indexB,
        endAMs: endA?.positionMs,
        endBMs: endB?.positionMs,
      });
      return;
    }

    // Native operation order: duration subtraction in float32, then
    // reciprocal followed by multiplication, each rounded to float32.
    const durationA = Math.fround(endA.positionSeconds - pointsA[indexA].positionSeconds);
    const durationB = Math.fround(endB.positionSeconds - pointsB[indexB].positionSeconds);
    const speedB = Math.fround(Math.fround(1 / durationA) * durationB);
    const difference = Math.abs(speedB - overlap.speedB);
    evaluateSpeedFormulas(trackA, pointsA[indexA], endA, trackB, pointsB[indexB], endB, overlap.speedB);
    maximumSpeedDifference = Math.max(maximumSpeedDifference, difference);
    if (Object.is(speedB, overlap.speedB)) {
      exactSpeedBits++;
      pairSummary.exactSpeedBits++;
    } else {
      discrepancies.push({
        pair: pair.label,
        rank,
        field: "speedB",
        expected: speedB,
        actual: overlap.speedB,
        absoluteDifference: difference,
      });
    }
  });
  perPair.push(pairSummary);
});

const hashes = [];
for (const fixture of trackCache.values()) {
  hashes.push({
    canonicalTrackUri: fixture.identity.canonicalTrackUri,
    playableTrackUri: fixture.identity.playableTrackUri,
    captured: fixture.identity.capturedBeatsHash,
    expected: fixture.identity.expectedPlayableBeatsHash,
    matches: fixture.identity.beatsHashMatchesExpected,
  });
}

console.log(JSON.stringify({
  format: "spotify-automix-input-validation-v1",
  oraclePairs: run.pairs.length,
  uniqueTracks: trackCache.size,
  totalTransitions: total,
  beatmatchedTransitions: beatmatched,
  nonBeatmatchedTransitions: total - beatmatched,
  exactGeometry,
  exactSpeedBits,
  maximumSpeedDifference,
  speedFormulaCandidates: [...speedFormulaStats.values()].sort((left, right) =>
    right.exact - left.exact || left.maximumDifference - right.maximumDifference
  ).slice(0, 12),
  exactPreset,
  beatsHashesMatched: hashes.filter((entry) => entry.matches).length,
  hashes,
  perPair,
  discrepancies,
  scoreValidation: {
    status: "not-yet-exact",
    reason: "The snapshots now contain all observed score inputs, but unresolved native base/cuepoint operation details remain outside this fixture-capture task.",
  },
}, null, 2));
