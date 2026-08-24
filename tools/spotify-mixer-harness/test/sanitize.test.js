const assert = require("assert");
const test = require("node:test");

const { sanitizeForCorpus, sanitizeLogText } = require("../lib/sanitize");

test("sanitizer redacts credential-shaped keys and bearer values", () => {
  const input = {
    headers: {
      authorization: "Bearer abc.def.ghi",
      cookie: "sp_dc=secret",
      statusCode: 200,
    },
    nested: {
      accessToken: "abc123",
      transition_uri: "spotify:transition:abc:1",
    },
    text: "prefix Bearer abc.def.ghi suffix",
  };

  const result = sanitizeForCorpus(input);

  assert.deepStrictEqual(result.value.headers, {
    authorization: "[REDACTED]",
    cookie: "[REDACTED]",
    statusCode: 200,
  });
  assert.strictEqual(result.value.nested.accessToken, "[REDACTED]");
  assert.strictEqual(result.value.nested.transition_uri, "spotify:transition:abc:1");
  assert.strictEqual(result.value.text, "prefix Bearer [REDACTED] suffix");
  assert.deepStrictEqual(result.redactedPaths.sort(), [
    "headers.authorization",
    "headers.cookie",
    "nested.accessToken",
    "text",
  ]);
});

test("spotifyd log sanitizer redacts account and connect identifiers", () => {
  const sanitized = sanitizeLogText(`
Restoring previous login as user alice.example.
Authenticated as 'alice.example' !
Using device id '77d890b023a23747e280d769a29fb238329a31cd'
Requesting https://gew4-spclient.spotify.com:443/connect-state/v1/devices/77d890b023a23747e280d769a29fb238329a31cd?product=0&country=DE&salt=4271300683
successfully put connect state for 77d890b023a23747e280d769a29fb238329a31cd
active device is <> with session <30351c15ab81425981bf25dab5cbb938>
couldn't parse packet from 192.168.2.210:5353
connection-id NzI0ZDYwMzUtMTBmOC00YWYyLWFiN2YtOGY3ZDUyOTQ3MmYzK2RlYWxlcit0Y3A6Ly8wYWIxNTAwOC5pcC5nZXc0LnNwb3RpZnkubmV0OjU3MDArRDg0MTI4QjFDQUQzRTdBMDNCRUM0N0I0NzMxNEM2MkY5MDY3MEMyQUU3OTNERERBNkM0NzZCMzE2RjcwNjEwQg==
Bearer abc.def.ghi
`);

  assert(!sanitized.includes("alice.example"));
  assert(!sanitized.includes("77d890b023a23747e280d769a29fb238329a31cd"));
  assert(!sanitized.includes("30351c15ab81425981bf25dab5cbb938"));
  assert(!sanitized.includes("4271300683"));
  assert(!sanitized.includes("192.168.2.210"));
  assert(!sanitized.includes("NzI0"));
  assert(!sanitized.includes("abc.def.ghi"));
});
