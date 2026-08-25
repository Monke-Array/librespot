const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const test = require("node:test");

const { buildSpotifydArgs, runSpotifydFor } = require("../lib/spotifyd");

test("spotifyd args include explicit device type when requested", () => {
  assert.deepStrictEqual(buildSpotifydArgs({ deviceType: "computer" }).slice(-2), [
    "--device-type",
    "computer",
  ]);
});

test("spotifyd runner enables materialized bypass and diagnostic env vars", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "spotifyd-runner-"));
  const debugDir = path.join(root, "target", "debug");
  fs.mkdirSync(debugDir, { recursive: true });
  const exe = path.join(debugDir, process.platform === "win32" ? "spotifyd.exe" : "spotifyd");
  fs.copyFileSync(process.execPath, exe);
  fs.chmodSync(exe, 0o755);

  const outDir = path.join(root, "runs");
  const result = await runSpotifydFor({
    spotifydRoot: root,
    outDir,
    durationMs: 5000,
    args: [
      "-e",
      "console.log(`${process.env.LIBRESPOT_DEV_BYPASS_MATERIALIZED_EQ}:${process.env.LIBRESPOT_DEV_BYPASS_MATERIALIZED_FILTER}:${process.env.LIBRESPOT_DEV_DUMP_MATERIALIZED_METADATA}`)",
    ],
  });

  assert.strictEqual(result.run.exitCode, 0);
  assert.deepStrictEqual(result.run.env, {
    LIBRESPOT_DEV_BYPASS_MATERIALIZED_EQ: "1",
    LIBRESPOT_DEV_BYPASS_MATERIALIZED_FILTER: "1",
    LIBRESPOT_DEV_DUMP_MATERIALIZED_METADATA: "1",
  });
  assert.match(result.run.stdout, /1:1:1/);
});

test("spotifyd runner records development capability override env", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "spotifyd-runner-"));
  const debugDir = path.join(root, "target", "debug");
  fs.mkdirSync(debugDir, { recursive: true });
  const exe = path.join(debugDir, process.platform === "win32" ? "spotifyd.exe" : "spotifyd");
  fs.copyFileSync(process.execPath, exe);
  fs.chmodSync(exe, 0o755);

  const outDir = path.join(root, "runs");
  const result = await runSpotifydFor({
    spotifydRoot: root,
    outDir,
    durationMs: 5000,
    env: {
      LIBRESPOT_DEV_CONNECT_CAPABILITY_OVERRIDES: "supports-dj=1",
    },
    args: [
      "-e",
      "console.log(process.env.LIBRESPOT_DEV_CONNECT_CAPABILITY_OVERRIDES)",
    ],
  });

  assert.strictEqual(result.run.exitCode, 0);
  assert.strictEqual(
    result.run.env.LIBRESPOT_DEV_CONNECT_CAPABILITY_OVERRIDES,
    "supports-dj=1",
  );
  assert.match(result.run.stdout, /supports-dj=1/);
});
