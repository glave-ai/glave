import { onTestFinished, vi } from 'vitest';

/** The object URLs a test's code made, in order, and those it let go of. */
export interface ObjectUrls {
  readonly made: readonly string[];
  readonly revoked: readonly string[];
}

/**
 * Object URLs for one test: each one made is new (`blob:test/1`, `blob:test/2`, …), and the test
 * sees which were made and let go of. `URL` is shared by every spec file in a worker, so these
 * are spies put back as the test finishes, never a stand-in left for the next file (#1282).
 */
export function objectUrls(): ObjectUrls {
  const made: string[] = [];
  const revoked: string[] = [];
  const create = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => {
    const url = `blob:test/${String(made.length + 1)}`;
    made.push(url);
    return url;
  });
  const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation((url) => {
    revoked.push(url);
  });
  onTestFinished(() => {
    create.mockRestore();
    revoke.mockRestore();
  });
  return { made, revoked };
}
