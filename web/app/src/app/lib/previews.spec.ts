import type { Preview } from '@glave/client';

import { command } from '../testing/fixtures';
import { LOOKS_AGAIN, frameable, previewKey, waitingForPreviews } from './previews';

function preview(url: string, overrides: Partial<Preview> = {}): Preview {
  return { command: 'c1', port: 5173, url, ...overrides };
}

describe('previewKey', () => {
  it('is its command and its port', () => {
    expect(previewKey(preview('http://127.0.0.1:61234/'))).toBe('c1:5173');
    expect(previewKey(preview('http://127.0.0.1:61234/', { port: 8080 }))).toBe('c1:8080');
  });
});

describe('waitingForPreviews', () => {
  it('is whether a command running shows no preview yet', () => {
    const dev = command('c1', 'npm run dev');
    const watch = command('c2', 'cargo watch');
    expect(waitingForPreviews([], [])).toBe(false);
    expect(waitingForPreviews([dev], [])).toBe(true);
    expect(waitingForPreviews([dev], [preview('http://127.0.0.1:61234/')])).toBe(false);
    expect(waitingForPreviews([dev, watch], [preview('http://127.0.0.1:61234/')])).toBe(true);
  });
});

describe('LOOKS_AGAIN', () => {
  it('looks again sooner first, and for a couple of minutes at most', () => {
    expect(LOOKS_AGAIN[0]).toBe(1_000);
    expect([...LOOKS_AGAIN].sort((a, b) => a - b)).toEqual(LOOKS_AGAIN);
    expect(LOOKS_AGAIN.reduce((sum, wait) => sum + wait, 0)).toBeLessThanOrEqual(150_000);
  });
});

describe('frameable', () => {
  it('is a page on this computer’s loopback at a port of its own, and nothing else', () => {
    expect(frameable(preview('http://127.0.0.1:61234/'))).toBe('http://127.0.0.1:61234/');
    for (const url of [
      'https://127.0.0.1:61234/',
      'http://localhost:61234/',
      'http://127.0.0.2:61234/',
      'http://127.0.0.1/',
      'http://someone@127.0.0.1:61234/',
      'javascript:alert(1)',
      'not a url',
      'http://example.com:61234/',
    ]) {
      expect(frameable(preview(url))).toBeNull();
    }
  });
});
