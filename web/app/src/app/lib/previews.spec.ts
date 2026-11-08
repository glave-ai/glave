import { command } from '../testing/fixtures';
import { LOOKS_AGAIN, previewKey, previewsOf } from './previews';

describe('previewKey', () => {
  it('is its command and its port', () => {
    expect(previewKey({ command: 'c1', port: 5173 })).toBe('c1:5173');
  });
});

describe('previewsOf', () => {
  it('is each port the server previews, of each command, in order', () => {
    const dev = command('c1', 'npm run dev', {
      ports: [
        { port: 5173, url: 'http://127.0.0.1:5173/', framed: true },
        { port: 9229, framed: false },
      ],
    });
    const docs = command('c2', 'npm run docs', {
      ports: [
        {
          port: 3000,
          url: 'http://[::1]:3000/',
          framed: false,
          unframed: 'It answers only at ::1.',
        },
      ],
    });
    expect(previewsOf([dev, command('c3', 'cargo watch'), docs])).toEqual([
      {
        command: 'c1',
        port: 5173,
        url: 'http://127.0.0.1:5173/',
        framed: true,
        unframed: null,
      },
      {
        command: 'c2',
        port: 3000,
        url: 'http://[::1]:3000/',
        framed: false,
        unframed: 'It answers only at ::1.',
      },
    ]);
    expect(previewsOf([])).toEqual([]);
  });
});

describe('LOOKS_AGAIN', () => {
  it('looks again sooner first, and for a couple of minutes at most', () => {
    expect(LOOKS_AGAIN[0]).toBe(1_000);
    expect([...LOOKS_AGAIN].sort((a, b) => a - b)).toEqual(LOOKS_AGAIN);
    expect(LOOKS_AGAIN.reduce((sum, wait) => sum + wait, 0)).toBeLessThanOrEqual(150_000);
  });
});
