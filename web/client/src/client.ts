// A small typed client of glave's session API over HTTP (specs/session.md, "Transports"):
// one JSON-RPC request per POST, the answer as JSON, or for a turn as server-sent events, its
// notifications first and the response last. No framework: only fetch and streams.

import {
  ErrorCode,
  STREAMING,
  type ErrorObject,
  type Method,
  type Methods,
  type Notification,
  type StreamingMethod,
} from './generated/api';

/** A request the server answered with an error: its JSON-RPC code, message and data. */
export class ApiError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(error: ErrorObject) {
    super(error.message);
    this.name = 'ApiError';
    this.code = error.code;
    this.data = error.data;
  }

  /** Whether the request failed for want of a valid sign-in. */
  get signedOut(): boolean {
    return this.code === ErrorCode.NO_SIGN_IN;
  }

  /** Whether it named an organization the caller is not in (any more). */
  get notInOrganization(): boolean {
    return this.code === ErrorCode.NOT_IN;
  }
}

/** How to reach one server. */
export interface ClientOptions {
  /** The server's address, such as `http://127.0.0.1:8080/`. */
  readonly url: string;
  /**
   * The bearer token for each request: a browser session, the server's local token, or an
   * OIDC access token; empty for none, as `server/discover` and `signin/redeem` need none.
   */
  readonly token: () => string | Promise<string>;
  /**
   * The organization each request works in, by its id, for someone who belongs to several
   * (`Glave-Organization`); none for the first they joined.
   */
  readonly organization?: () => string | null;
  /** The fetch to use; the global one by default. */
  readonly fetch?: typeof fetch;
}

/** One call's params, or nothing for a method that takes none. */
type Params<M extends Method> = Methods[M]['params'];
type Result<M extends Method> = Methods[M]['result'];

/** What a streaming call yields: each notification as it arrives. Its return is the result. */
export type TurnStream<M extends StreamingMethod> = AsyncGenerator<Notification, Result<M>, void>;

/** How long a call waits for the server's answer: reading or changing what it holds takes moments. */
const ANSWERS_WITHIN_MS = 2 * 60_000;

/**
 * How long the slow calls wait: those that ask a model (compacting, checking a provider), reach
 * another service (a git host, a connection), make a computer to work on or a folder, or send a
 * file; and a turn or goal to start, whose stream begins once it has.
 */
const SLOW_ANSWERS_WITHIN_MS = 15 * 60_000;

/** The calls that wait `SLOW_ANSWERS_WITHIN_MS`, as glave's own client waits for them. */
const SLOW: ReadonlySet<Method> = new Set<Method>([
  'connection/start',
  'credential/connect',
  'file/upload',
  'goal/start',
  'project/create',
  'provider/connect',
  'provider/refresh',
  'thread/compact',
  'thread/start',
  'turn/start',
]);

/**
 * How long a stream may go without a word: the server says something at least every 15
 * seconds while it streams, so a silence of four times that is a lost stream, as on a
 * connection a change of network left dead.
 */
const STREAM_IDLE_MS = 60_000;

/** A client of one server. */
export class GlaveClient {
  private readonly options: ClientOptions;
  private nextId = 0;

  constructor(options: ClientOptions) {
    this.options = options;
  }

  /**
   * Calls `method` and resolves with its result; a turn's notifications are passed over. It
   * waits 2 minutes for the answer, or 15 for a call that waits on a model, another service, a
   * computer or a file, then throws that the server did not answer.
   */
  async call<M extends Method>(
    method: M,
    params: Params<M>,
    signal?: AbortSignal,
  ): Promise<Result<M>> {
    return within(answersWithin(method), signal, async (bounded) => {
      const response = await this.post(method, params, bounded);
      if (!isEventStream(response)) {
        return answerOf<Result<M>>(await response.json());
      }
      for await (const message of messages(response)) {
        if (!isNotification(message)) {
          return answerOf<Result<M>>(message);
        }
      }
      throw new ApiError({
        code: ErrorCode.INTERNAL,
        message: 'the stream ended without an answer',
      });
    });
  }

  /**
   * Starts a turn or a goal and yields its notifications as they arrive; the generator returns
   * the turn's result. Closing it early stops only this stream: the turn goes on in the server,
   * and `thread/events` or `thread/watch` pick it up again. It waits for the stream to begin as
   * `call` waits for an answer, and throws once the stream says nothing for a minute, as a lost
   * one does.
   */
  async *stream<M extends StreamingMethod>(
    method: M,
    params: Params<M>,
    signal?: AbortSignal,
  ): TurnStream<M> {
    const response = await within(answersWithin(method), signal, (bounded) =>
      this.post(method, params, bounded),
    );
    if (!isEventStream(response)) {
      return answerOf<Result<M>>(await response.json());
    }
    for await (const message of messages(response)) {
      if (isNotification(message)) {
        yield message;
      } else {
        return answerOf<Result<M>>(message);
      }
    }
    throw new ApiError({ code: ErrorCode.INTERNAL, message: 'the stream ended without an answer' });
  }

  private async post(method: Method, params: unknown, signal: AbortSignal): Promise<Response> {
    this.nextId += 1;
    const body = JSON.stringify({ jsonrpc: '2.0', id: this.nextId, method, params });
    const token = await this.options.token();
    const send = this.options.fetch ?? fetch;
    const streams: readonly string[] = STREAMING;
    const init: RequestInit = {
      method: 'POST',
      body,
      headers: {
        'Content-Type': 'application/json',
        Accept: streams.includes(method)
          ? 'text/event-stream, application/json'
          : 'application/json',
        ...(token === '' ? {} : { Authorization: `Bearer ${token}` }),
        ...organizationHeader(this.options.organization?.() ?? null),
        'Glave-Method': method,
      },
      signal,
    };
    return send(this.options.url, init);
  }
}

/**
 * The one-time code in a link glave opened or printed, if the fragment has one: a sign-in
 * link's (`…/#code=<code>`), which `signin/redeem` trades for a browser session, a setup
 * link's (`…/#setup=<code>`), with which `passkey/create_options` makes the first passkey, or
 * an invite's (`…/#invite=<code>`), which `invite/show` says what it opens, a sign-up's
 * (`…/#signup=<code>`), emailed, with which `passkey/create_options` makes a new account, or
 * one to get back in (`…/#recover=<code>`), emailed, with which it makes a new passkey for the
 * account the address has.
 */
export function codeFromFragment(
  fragment: string,
  link: 'code' | 'setup' | 'invite' | 'signup' | 'recover',
): string | undefined {
  const fields = new URLSearchParams(fragment.startsWith('#') ? fragment.slice(1) : fragment);
  const code = fields.get(link);
  return code === null || code === '' ? undefined : code;
}

/** The header that names `organization`, when there is one. */
function organizationHeader(organization: string | null): Record<string, string> {
  return organization === null ? {} : { 'Glave-Organization': organization };
}

function isEventStream(response: Response): boolean {
  return (response.headers.get('Content-Type') ?? '').startsWith('text/event-stream');
}

function isObject(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNotification(message: unknown): message is Notification {
  return isObject(message) && typeof message['method'] === 'string' && !('id' in message);
}

function isErrorObject(value: unknown): value is ErrorObject {
  return (
    isObject(value) && typeof value['code'] === 'number' && typeof value['message'] === 'string'
  );
}

/**
 * The result of a JSON-RPC response, or its error thrown as an `ApiError`. The caller names
 * the result's type, which the schema the server's types come from guarantees.
 */
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- see above
function answerOf<T>(message: unknown): T {
  if (!isObject(message)) {
    throw new ApiError({ code: ErrorCode.PARSE, message: 'the answer is not a JSON-RPC response' });
  }
  const error = message['error'];
  if (error !== undefined) {
    throw new ApiError(
      isErrorObject(error) ? error : { code: ErrorCode.INTERNAL, message: 'the server failed' },
    );
  }
  if (!('result' in message)) {
    throw new ApiError({ code: ErrorCode.PARSE, message: 'the answer has no result' });
  }
  // The server's types guarantee the shape; the schema it was generated from says so.
  return message['result'] as T;
}

/** How long `method` waits for its answer, or for its stream to begin. */
function answersWithin(method: Method): number {
  return SLOW.has(method) ? SLOW_ANSWERS_WITHIN_MS : ANSWERS_WITHIN_MS;
}

/** `ms` in words: `a minute`, `15 minutes`. */
function spoken(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  return minutes === 1 ? 'a minute' : `${String(minutes)} minutes`;
}

/**
 * What `ask` answers, given a signal that aborts with `signal` or once `ms` have passed; then it
 * throws that the server did not answer within that time, whether or not `ask` heeds the
 * signal. Once `ask` has answered, the signal stays live, for the body of a stream it began.
 */
async function within<T>(
  ms: number,
  signal: AbortSignal | undefined,
  ask: (bounded: AbortSignal) => Promise<T>,
): Promise<T> {
  const limit = new AbortController();
  const bounded = signal === undefined ? limit.signal : AbortSignal.any([signal, limit.signal]);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      // Said first, so it is what the call throws, not what the abort makes of it.
      reject(
        new ApiError({
          code: ErrorCode.INTERNAL,
          message: `the server did not answer within ${spoken(ms)}`,
        }),
      );
      limit.abort();
    }, ms);
  });
  try {
    return await Promise.race([ask(bounded), late]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * The stream's next chunk, or a throw once it has said nothing for `STREAM_IDLE_MS`, the stream
 * cancelled so its connection goes.
 */
async function nextChunk(
  reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const quiet = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      // Said first, so it is what the read throws, not the end the cancelling makes.
      reject(
        new ApiError({
          code: ErrorCode.INTERNAL,
          message: `the server said nothing for ${spoken(STREAM_IDLE_MS)}`,
        }),
      );
      reader.cancel().catch(() => undefined);
    }, STREAM_IDLE_MS);
  });
  try {
    return await Promise.race([reader.read(), quiet]);
  } finally {
    clearTimeout(timer);
  }
}

/** Each server-sent event's message, parsed, as it arrives, until the stream goes quiet. */
async function* messages(response: Response): AsyncGenerator<unknown, void, void> {
  const body = response.body;
  if (body === null) {
    return;
  }
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    for (;;) {
      const { done, value } = await nextChunk(reader);
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      let end = buffer.indexOf('\n\n');
      while (end >= 0) {
        const event = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const data = event
          .split('\n')
          .filter((line) => line.startsWith('data:'))
          .map((line) => line.slice(5).trimStart())
          .join('\n');
        if (data !== '') {
          yield JSON.parse(data) as unknown;
        }
        end = buffer.indexOf('\n\n');
      }
      if (done) {
        return;
      }
    }
  } finally {
    reader.releaseLock();
  }
}
