import { type IncomingMessage, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

/** The model the fake service runs. */
export const LIVE_MODEL = 'fake-live';

/** What it says to anything but a request to touch a file. */
export const HELLO = 'Hello from the live model.';

/** What it says once its call to touch a file has a result. */
export const LEFT_ALONE = 'Left it alone.';

/** The command it asks to run when asked to touch a file. */
export const TOUCH = 'touch live.txt';

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

/** A reply asking to run `command`. */
function run(response: ServerResponse, command: string): void {
  const call = {
    index: 0,
    id: 'call_live',
    type: 'function',
    function: { name: 'bash', arguments: JSON.stringify({ command }) },
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
    if (last?.role === 'tool') {
      say(response, LEFT_ALONE);
    } else if (textOf(last?.content).toLowerCase().includes('touch')) {
      run(response, TOUCH);
    } else {
      say(response, HELLO);
    }
  });
}

/**
 * Starts a model service a real glave talks to in the end-to-end tests (live.e2e.ts), on a port
 * the system picks, and answers its OpenAI-compatible address. Asked to touch a file, it asks to
 * run `touch`; once that call has a result, it says it left the file alone; to anything else, it
 * says hello. It does not keep the process alive.
 */
export async function startFakeModel(): Promise<string> {
  const server = createServer(answer);
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  server.unref();
  const { port } = server.address() as AddressInfo;
  return `http://127.0.0.1:${String(port)}/v1`;
}
