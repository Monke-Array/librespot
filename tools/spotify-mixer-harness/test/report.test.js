const assert = require("assert");
const test = require("node:test");

const { summarizeCoverage } = require("../lib/report");

test("coverage report counts statuses, signatures, presets, and largest blocker", () => {
  const report = summarizeCoverage([
    {
      signature: { hash: "sig-a", summary: { presetId: "1", effects: ["eq"], transitionMs: 6090 } },
      classification: { status: "DEV-EQ-BYPASS", blockers: [] },
    },
    {
      signature: { hash: "sig-b", summary: { presetId: "2", effects: ["filter"], transitionMs: 8000 } },
      classification: { status: "UNSUPPORTED-FILTER", blockers: ["filter"] },
    },
    {
      signature: { hash: "sig-b", summary: { presetId: "2", effects: ["filter"], transitionMs: 8000 } },
      classification: { status: "UNSUPPORTED-FILTER", blockers: ["filter"] },
    },
  ]);

  assert.strictEqual(report.transitionsSampled, 3);
  assert.strictEqual(report.distinctDspSignatures, 2);
  assert.deepStrictEqual(report.statusCounts, {
    "DEV-EQ-BYPASS": 1,
    "UNSUPPORTED-FILTER": 2,
  });
  assert.deepStrictEqual(report.presetCoverage, { 1: 1, 2: 2 });
  assert.deepStrictEqual(report.largestBlockers, [{ blocker: "filter", count: 2 }]);
  assert.strictEqual(report.supportedCorpusPercentage, 33.33);
});

