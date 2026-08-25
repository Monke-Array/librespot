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
      classification: { status: "DEV-FILTER-BYPASS", blockers: [] },
    },
    {
      signature: { hash: "sig-b", summary: { presetId: "2", effects: ["filter"], transitionMs: 8000 } },
      classification: { status: "DEV-EQ-FILTER-BYPASS", blockers: [] },
    },
  ]);

  assert.strictEqual(report.transitionsSampled, 3);
  assert.strictEqual(report.distinctDspSignatures, 2);
  assert.deepStrictEqual(report.statusCounts, {
    "DEV-EQ-BYPASS": 1,
    "DEV-FILTER-BYPASS": 1,
    "DEV-EQ-FILTER-BYPASS": 1,
  });
  assert.deepStrictEqual(report.presetCoverage, { 1: 1, 2: 2 });
  assert.deepStrictEqual(report.largestBlockers, []);
  assert.strictEqual(report.supportedCorpusPercentage, 100);
});
