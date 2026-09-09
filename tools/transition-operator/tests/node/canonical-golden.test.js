const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const fixtures = path.join(__dirname, '..', 'fixtures', 'plans');
const golden = JSON.parse(fs.readFileSync(path.join(fixtures, 'golden-identities.json'), 'utf8'));

function canonical(value) {
  if (value === null || typeof value === 'number' && (!Number.isSafeInteger(value))) {
    throw new Error('non-integer or null canonical value');
  }
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (typeof value === 'object') {
    const keys = Object.keys(value).sort((a, b) => Buffer.from(a).compare(Buffer.from(b)));
    if (keys.some((key) => !/^[\x00-\x7f]*$/.test(key))) throw new Error('non-ASCII key');
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`;
  }
  if (typeof value === 'string') return JSON.stringify(value);
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'number') return String(value);
  throw new Error('unsupported canonical value');
}

function sha256(bytes) { return crypto.createHash('sha256').update(bytes).digest('hex'); }

function projections(plan) {
  const body = structuredClone(plan); delete body.plan_id;
  const audio = Object.fromEntries(['schema_version', 'format', 'sources', 'timeline', 'operations', 'output_safety'].map((key) => [key, plan[key]]));
  const planHash = sha256(Buffer.from(canonical(body)));
  const candidateHash = sha256(Buffer.concat([Buffer.from('transition-candidate/1\0'), Buffer.from(planHash, 'hex')]));
  return { planHash, audioHash: sha256(Buffer.from(canonical(audio))), candidateHash };
}

function capabilities(plan) {
  const required = new Set(['format:pcm_f64_stereo_44100_v1', 'limiter:lookahead_peak_limiter_v1', 'source:pcm_s16le_stereo_44100_v1', 'output_safety:transition_output_safety_v1', 'plan:transition-operator-plan/1', 'true_peak:bs1770_4x_v1']);
  let maxEnvelope = 0, maxBands = 0, maxTaps = 0, maxState = plan.output_safety.lookahead_frames, maxRate = 0;
  const addEnvelope = (points, interpolations) => {
    maxEnvelope = Math.max(maxEnvelope, points.length);
    for (const interpolation of interpolations) required.add(`interpolation:${interpolation}`);
  };
  for (const operation of plan.operations) {
    required.add(`operation:${operation.kind}`);
    if (operation.kind === 'time_map') {
      required.add(`profile:${operation.profile}`); required.add(`time_stretch_rate_ppm:${operation.source_rate_ppm}`);
      maxRate = Math.abs(operation.source_rate_ppm - 1000000);
    } else if (operation.kind === 'gain_envelope' || operation.kind === 'duck_envelope' || operation.kind === 'rhythmic_gate') {
      addEnvelope(operation.points, operation.interpolations);
    } else if (operation.kind === 'filter_envelope') {
      required.add(`profile:${operation.filter_kind}`); maxEnvelope = Math.max(maxEnvelope, operation.cutoff_points.length, operation.wet_points.length);
      for (const interpolation of [...operation.cutoff_interpolations, ...operation.wet_interpolations]) required.add(`interpolation:${interpolation}`);
    } else if (operation.kind === 'crossover_band_gain') {
      required.add(`profile:${operation.profile}`); maxBands = Math.max(maxBands, operation.bands.length);
      for (const envelope of operation.band_gain_envelopes) addEnvelope(envelope.points, envelope.interpolations);
      addEnvelope(operation.wet_points, operation.wet_interpolations);
    } else if (operation.kind === 'feedforward_delay_tail') {
      maxTaps = Math.max(maxTaps, operation.taps.length); maxState = Math.max(maxState, operation.taps.at(-1).delay_frames);
    }
  }
  required.add(`limit:max_abs_rate_delta_ppm:${maxRate}`); required.add(`limit:max_bands:${maxBands}`);
  required.add(`limit:max_envelope_points:${maxEnvelope}`); required.add(`limit:max_state_span_frames:${maxState}`);
  required.add(`limit:max_taps:${maxTaps}`); required.add(`lookahead_frames:${plan.output_safety.lookahead_frames}`);
  return { required: [...required].sort(), max_envelope_points: maxEnvelope, max_bands: maxBands, max_taps: maxTaps, max_state_span_frames: maxState, max_abs_rate_delta_ppm: maxRate, lookahead_frames: plan.output_safety.lookahead_frames };
}

test('independent canonical and hash vectors match', () => {
  assert.equal(golden.max_safe_integer, Number.MAX_SAFE_INTEGER);
  for (const vector of golden.vectors) {
    const raw = fs.readFileSync(path.join(fixtures, vector.fixture), 'utf8').replace(/\n$/, '');
    const plan = JSON.parse(raw); const canonicalBytes = canonical(plan);
    assert.equal(canonicalBytes, raw, `${vector.fixture} canonical bytes`);
    const hashes = projections(plan);
    assert.equal(sha256(Buffer.from(canonicalBytes)), vector.canonical_sha256);
    assert.equal(hashes.planHash, vector.plan_sha256);
    assert.equal(hashes.audioHash, vector.audio_semantics_sha256);
    assert.equal(`op1-${hashes.planHash}`, vector.plan_id);
    assert.equal(`cand1-${hashes.candidateHash}`, vector.candidate_id);
    assert.deepEqual(capabilities(plan), vector.capabilities);
  }
});
