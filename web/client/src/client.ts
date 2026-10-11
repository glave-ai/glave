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
} from "./generated/api";

/** A request the server answered with an error: its JSON-RPC code, message and data. */
export class ApiError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(error: ErrorObject) {
    super(error.message);
    this.name = "ApiError";
    this.code = error.code;
    this.data = error.data;
  }

  /** Whether the request failed for want of a valid sign-in. */
  get signedOut(): boolean {
    return this.code === ErrorCode.NO_SIGN_IN;
  }
}

/** How to reach one server. */
export interface ClientOptions {
  /** The server's address, such as `http://127.0.0.1:8080/`. */
  readonly url: string;
  /** The bearer token for each request: the server's local token, or an OIDC access token. */
  readonly token: () => string | Promise<string>;
  /** The fetch to use; the global one by default. */
  readonly fetch?: typeof fetch;
}

/** One call's params, or nothing for a method that takes none. */
type Params<M extends Method> = Methods[M]["params"];
type Result<M extends Method> = Methods[M]["result"];

/** What a streaming call yields: each notification as it arrives. Its return is the result. */
export type TurnStream<M extends StreamingMethod> = AsyncGenerator<Notification, Result<M>, void>;

/** A client of one server. */
export class GlaveClient {
  private readonly options: ClientOptions;
  private nextId = 0;

  constructor(options: ClientOptions) {
    this.options = options;
  }

  /** Calls `method` and resolves with its result; a turn's notifications are passed over. */
  async call<M extends Method>(
    method: M,
    params: Params<M>,
    signal?: AbortSignal,
  ): Promise<Result<M>> {
    const response = await this.post(method, params, signal);
    if (!isEventStream(response)) {
      return answerOf<Result<M>>(await response.json());
    }
    for await (const message of messages(response)) {
      if (!isNotification(message)) {
        return answerOf<Result<M>>(message);
      }
    }
    throw new ApiError({ code: ErrorCode.INTERNAL, message: "the stream ended without an answer" });
  }

  /**
   * Starts a turn or a goal and yields its notifications as they arrive; the generator returns
   * the turn's result. Closing it early stops only this stream: the turn goes on in the server,
   * and `thread/events` or `thread/watch` pick it up again.
   */
  async *stream<M extends StreamingMethod>(
    method: M,
    params: Params<M>,
    signal?: AbortSignal,
  ): TurnStream<M> {
    const response = await this.post(method, params, signal);
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
    throw new ApiError({ code: ErrorCode.INTERNAL, message: "the stream ended without an answer" });
  }

  private async post(method: Method, params: unknown, signal?: AbortSignal): Promise<Response> {
    this.nextId += 1;
    const body = JSON.stringify({ jsonrpc: "2.0", id: this.nextId, method, params });
    const token = await this.options.token();
    const send = this.options.fetch ?? fetch;
    const streams: readonly string[] = STREAMING;
    const init: RequestInit = {
      method: "POST",
      body,
      headers: {
        "Content-Type": "application/json",
        Accept: streams.includes(method) ? "text/event-stream, application/json" : "application/json",
        Authorization: `Bearer ${token}`,
        "Glave-Method": method,
      },
      ...(signal === undefined ? {} : { signal }),
    };
    return send(this.options.url, init);
  }
}

/** The token in a link `glave serve` printed (`…/#token=<token>`), if the fragment has one. */
export function tokenFromFragment(fragment: string): string | undefined {
  const fields = new URLSearchParams(fragment.startsWith("#") ? fragment.slice(1) : fragment);
  const token = fields.get("token");
  return token === null || token === "" ? undefined : token;
}

function isEventStream(response: Response): boolean {
  return (response.headers.get("Content-Type") ?? "").startsWith("text/event-stream");
}

function isObject(value: unknown): value is { readonly [key: string]: unknown } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNotification(message: unknown): message is Notification {
  return isObject(message) && typeof message["method"] === "string" && !("id" in message);
}

function isErrorObject(value: unknown): value is ErrorObject {
  return isObject(value) && typeof value["code"] === "number" && typeof value["message"] === "string";
}

/** The result of a JSON-RPC response, or its error thrown as an `ApiError`. */
function answerOf<T>(message: unknown): T {
  if (!isObject(message)) {
    throw new ApiError({ code: ErrorCode.PARSE, message: "the answer is not a JSON-RPC response" });
  }
  const error = message["error"];
  if (error !== undefined) {
    throw new ApiError(
      isErrorObject(error) ? error : { code: ErrorCode.INTERNAL, message: "the server failed" },
    );
  }
  if (!("result" in message)) {
    throw new ApiError({ code: ErrorCode.PARSE, message: "the answer has no result" });
  }
  // The server's types guarantee the shape; the schema it was generated from says so.
  return message["result"] as T;
}

/** Each server-sent event's message, parsed, as it arrives. */
async function* messages(response: Response): AsyncGenerator<unknown, void, void> {
  const body = response.body;
  if (body === null) {
    return;
  }
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const { done, value } = await reader.read();
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      let end = buffer.indexOf("\n\n");
      while (end >= 0) {
        const event = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const data = event
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (data !== "") {
          yield JSON.parse(data) as unknown;
        }
        end = buffer.indexOf("\n\n");
      }
      if (done) {
        return;
      }
    }
  } finally {
    reader.releaseLock();
  }
}
