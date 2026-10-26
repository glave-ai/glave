import { ApiError, GlaveClient } from './client';

/** A body the test writes to as the server would, and whether the client cancelled it. */
interface Body {
  readonly stream: ReadableStream<Uint8Array>;
  say(text: string): void;
  readonly cancelled: () => boolean;
}

function body(): Body {
  const encoder = new TextEncoder();
  let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
  let cancelled = false;
  const stream = new ReadableStream<Uint8Array>({
    start: (started) => {
      controller = started;
    },
    cancel: () => {
      cancelled = true;
    },
  });
  return {
    stream,
    say: (text) => {
      controller?.enqueue(encoder.encode(text));
    },
    cancelled: () => cancelled,
  };
}

/** A client whose every request gets `answer`, or, with none, no answer ever: only an abort. */
function clientOf(answer?: () => Response): GlaveClient {
  const fetch = (_: RequestInfo | URL, init?: RequestInit): Promise<Response> =>
    new Promise((resolve, reject) => {
      const signal = init?.signal;
      const aborted = (): void => {
        reject(new DOMException('The operation was aborted.', 'AbortError'));
      };
      if (signal?.aborted === true) {
        aborted();
      }
      signal?.addEventListener('abort', aborted);
      if (answer !== undefined) {
        resolve(answer());
      }
    });
  return new GlaveClient({ url: 'http://glave.test/', token: () => '', fetch });
}

/** How `promise` has settled so far, so a test can look before and after the time it waits. */
function settled(
  promise: Promise<unknown>,
): () => { readonly done: boolean; readonly error: unknown } {
  let state: { done: boolean; error: unknown } = { done: false, error: undefined };
  promise.then(
    () => {
      state = { done: true, error: undefined };
    },
    (error: unknown) => {
      state = { done: true, error };
    },
  );
  return () => state;
}

const NOTE = 'data: {"jsonrpc":"2.0","method":"thread/changed","params":{}}\n\n';

describe('GlaveClient', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('gives up on a stream that says nothing for a minute, and cancels it', async () => {
    const server = body();
    const client = clientOf(
      () => new Response(server.stream, { headers: { 'Content-Type': 'text/event-stream' } }),
    );
    const watch = client.stream('thread/list/watch', {});
    server.say(NOTE);
    expect((await watch.next()).done).toBe(false);
    const next = settled(watch.next());
    // A keep-alive is a word: the minute counts again from it.
    await vi.advanceTimersByTimeAsync(45_000);
    server.say(': keep-alive\n\n');
    await vi.advanceTimersByTimeAsync(59_000);
    expect(next().done).toBe(false);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(next().error).toBeInstanceOf(ApiError);
    expect(next().error).toHaveProperty('message', 'the server said nothing for a minute');
    expect(server.cancelled()).toBe(true);
  });

  it('gives up on a call the server never answers, within its kind of time', async () => {
    const client = clientOf();
    const read = settled(client.call('thread/get', { thread: 'thread-1' }));
    await vi.advanceTimersByTimeAsync(119_000);
    expect(read().done).toBe(false);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(read().error).toHaveProperty('message', 'the server did not answer within 2 minutes');
    // A file takes the server longer.
    const upload = settled(
      client.call('file/upload', { name: 'a.txt', media_type: 'text/plain', data: '' }),
    );
    await vi.advanceTimersByTimeAsync(14 * 60_000);
    expect(upload().done).toBe(false);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(upload().error).toHaveProperty('message', 'the server did not answer within 15 minutes');
  });

  it('waits for a turn to begin as long as for a slow call', async () => {
    const client = clientOf();
    const turn = settled(client.stream('turn/start', { thread: 'thread-1', prompt: 'hi' }).next());
    await vi.advanceTimersByTimeAsync(14 * 60_000);
    expect(turn().done).toBe(false);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(turn().error).toHaveProperty('message', 'the server did not answer within 15 minutes');
  });

  it('stops waiting at once when the caller stops it', async () => {
    const client = clientOf();
    const stop = new AbortController();
    const read = settled(client.call('thread/get', { thread: 'thread-1' }, stop.signal));
    await vi.advanceTimersByTimeAsync(0);
    stop.abort();
    await vi.advanceTimersByTimeAsync(0);
    expect(read().error).toHaveProperty('name', 'AbortError');
  });
});
