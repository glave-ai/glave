import { test } from "node:test";
import assert from "node:assert/strict";
import { buildUrl } from "./url.ts";

test("no params returns base", () => {
  assert.equal(buildUrl("/search", {}), "/search");
});

test("plain param is appended", () => {
  assert.equal(buildUrl("/search", { a: "1" }), "/search?a=1");
});

test("encodes param values with spaces", () => {
  assert.equal(buildUrl("/search", { q: "a b" }), "/search?q=a%20b");
});
