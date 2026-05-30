/** Build a URL with a query string from a base path and params. */
export function buildUrl(base: string, params: Record<string, string>): string {
  const parts = Object.entries(params).map(
    ([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`,
  );
  return parts.length ? `${base}?${parts.join("&")}` : base;
}
