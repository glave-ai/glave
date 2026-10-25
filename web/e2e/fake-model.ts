import { type IncomingMessage, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

/** The model the fake service runs. */
export const LIVE_MODEL = 'fake-live';

/** What it says to anything but a request to touch a file. */
export const HELLO = 'Hello from the live model.';

/** What it says once its call to touch a file has a result. */
export const LEFT_ALONE = 'Left it alone.';

/**
 * What it asks to change when asked to touch a file: writing `live.txt`, a change that asks in
 * Read-only, as the approval shows it.
 */
export const TOUCH = 'live.txt (1 lines)';

/**
 * What, said in a prompt, holds the fake's reply to it until a test lets it go (`letGo`): so a
 * test sees a turn while it runs, for as long as it needs, with no timer.
 */
export function holding(name: string): string {
  return `Hold ${name}`;
}

/** The name a prompt holds its reply by, if it does. */
function heldBy(text: string): string | undefined {
  return /\bHold ([\w-]+)/u.exec(text)?.[1];
}

/** Lets go of the replies the fake at `service` holds for `name`, and holds none after. */
export async function letGo(service: string, name: string): Promise<void> {
  const response = await fetch(`${service}/let-go/${encodeURIComponent(name)}`, {
    method: 'POST',
  });
  if (!response.ok) {
    throw new Error(`the fake model did not let go of ${name}`);
  }
}

/** The replies held, by the name that holds them, and the names let go of. */
const held = new Map<string, (() => void)[]>();
const freed = new Set<string>();

/** `chunks` as a stream of server-sent events, ending as OpenAI's do. */
function stream(response: ServerResponse, chunks: readonly unknown[]): void {
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  const events = chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join('');
  response.end(`${events}data: [DONE]\n\n`);
}

const USAGE = { prompt_tokens: 10, completion_tokens: 5 };

/** A reply saying `text`. */
function say(response: ServerResponse, text: string): void {
  stream(response, [
    { id: 'gen-say', model: LIVE_MODEL, choices: [{ index: 0, delta: { content: text } }] },
    { choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: USAGE },
  ]);
}

/** A reply asking to write `live.txt`. */
function write(response: ServerResponse): void {
  const call = {
    index: 0,
    id: 'call_live',
    type: 'function',
    function: { name: 'write', arguments: JSON.stringify({ path: 'live.txt', content: 'live\n' }) },
  };
  stream(response, [
    { id: 'gen-run', model: LIVE_MODEL, choices: [{ index: 0, delta: { tool_calls: [call] } }] },
    { choices: [{ index: 0, delta: {}, finish_reason: 'tool_calls' }], usage: USAGE },
  ]);
}

interface Message {
  readonly role?: unknown;
  readonly content?: unknown;
}

/** The text of a chat message's content: a string, or a list of parts. */
function textOf(content: unknown): string {
  if (typeof content === 'string') {
    return content;
  }
  if (!Array.isArray(content)) {
    return '';
  }
  return (content as readonly unknown[])
    .map((part) =>
      typeof part === 'object' && part !== null && 'text' in part && typeof part.text === 'string'
        ? part.text
        : '',
    )
    .join('');
}

/** Answers one request as an OpenAI-compatible service would. */
function answer(request: IncomingMessage, response: ServerResponse): void {
  const freeing = /^\/v1\/let-go\/(.+)$/u.exec(request.url ?? '')?.[1];
  if (request.method === 'POST' && freeing !== undefined) {
    const name = decodeURIComponent(freeing);
    freed.add(name);
    for (const reply of held.get(name) ?? []) {
      reply();
    }
    held.delete(name);
    response.writeHead(204);
    response.end();
    return;
  }
  if (request.method === 'GET' && request.url === '/v1/models') {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ object: 'list', data: [{ id: LIVE_MODEL, created: 1 }] }));
    return;
  }
  let body = '';
  request.setEncoding('utf8');
  request.on('data', (chunk: string) => {
    body += chunk;
  });
  request.on('end', () => {
    let last: Message | undefined;
    try {
      const parsed = JSON.parse(body) as { readonly messages?: readonly Message[] };
      last = parsed.messages?.at(-1);
    } catch {
      response.writeHead(400);
      response.end();
      return;
    }
    const text = textOf(last?.content);
    const reply = (): void => {
      if (last?.role === 'tool') {
        say(response, LEFT_ALONE);
      } else if (text.toLowerCase().includes('touch')) {
        write(response);
      } else {
        say(response, HELLO);
      }
    };
    const name = last?.role === 'tool' ? undefined : heldBy(text);
    if (name === undefined || freed.has(name)) {
      reply();
    } else {
      held.set(name, [...(held.get(name) ?? []), reply]);
    }
  });
}

/**
 * Starts a model service a real glave talks to in the end-to-end tests (live.e2e.ts), on a port
 * the system picks, and answers its OpenAI-compatible address. Asked to touch a file, it asks to
 * write one; once that call has a result, it says it left the file alone; to anything else, it
 * says hello. A prompt that holds its reply (`holding`) waits for the test to let it go. It does
 * not keep the process alive.
 */
export async function startFakeModel(): Promise<string> {
  const server = createServer(answer);
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  server.unref();
  const { port } = server.address() as AddressInfo;
  return `http://127.0.0.1:${String(port)}/v1`;
}
