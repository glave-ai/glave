/** Build a URL with a query string from a base path and params. */
export function buildUrl(base: string, params: Record<string, string>): string {
  // BUG: param values are not URL-encoded, so spaces and special characters
  // produce a malformed URL.
  const parts = Object.entries(params).map(([k, v]) => `${k}=${v}`);
  return parts.length ? `${base}?${parts.join("&")}` : base;
}
