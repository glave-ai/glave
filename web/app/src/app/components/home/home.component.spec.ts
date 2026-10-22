import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type TurnStartParams } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { ProjectsService } from '../../services/projects.service';
import { ELSEWHERE, FakeSession, FakeTurn, provideFakeSession } from '../../testing/fake-session';
import { MODELS, SERVER, event, record, thread } from '../../testing/fixtures';
import { HomeComponent } from './home.component';

/** Any idempotency key, as the app makes one. */
const KEY = expect.stringMatching(/^[0-9a-f]{32}$/u) as unknown;

async function send(root: HTMLElement, text: string): Promise<void> {
  const input = root.querySelector<HTMLTextAreaElement>('[data-testid="home-composer-input"]');
  if (input) {
    input.value = text;
    input.dispatchEvent(new Event('input'));
  }
  await Promise.resolve();
  input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
}

function session(): FakeSession {
  return new FakeSession()
    .answer('project/list', {
      projects: [
        record('p1', 'glave'),
        record('p2', 'Garden'),
        record('me', 'Personal', 'personal'),
      ],
    })
    .answer('thread/list', { threads: [] })
    .answer('thread/start', { thread: thread('t9', null) })
    .answer('model/list', { models: MODELS })
    .answer('server/discover', SERVER);
}

function where(root: HTMLElement): string {
  const trigger = root.querySelector('[data-testid="home-where-trigger"]');
  return trigger?.textContent.replace(/\s+/gu, ' ').trim() ?? '';
}

async function render(fake: FakeSession, projectId?: string, narrow = false): Promise<HTMLElement> {
  const breakpoints = { observe: () => of({ matches: narrow, breakpoints: {} }) };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      { provide: BreakpointObserver, useValue: breakpoints },
    ],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(HomeComponent);
  if (projectId !== undefined) {
    fixture.componentRef.setInput('project', projectId);
  }
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('HomeComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('starts a thread in the personal project, then its first turn, then opens it', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    expect(root.querySelector('h1')?.textContent).toBe('What should we work on?');
    expect(root.querySelector('[data-testid="home-composer"]')?.classList).toContain(
      'glz-composer--lg',
    );
    expect(root.querySelector('[data-testid="approval-mode"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="model-picker-trigger"]')?.textContent).toContain(
      'GPT-6 Luna',
    );
    expect(where(root)).toBe('Where: A chat');
    expect(document.activeElement).not.toBe(
      root.querySelector('[data-testid="home-composer-input"]'),
    );
    // The Chats is where Back leads, so it has none.
    expect(root.querySelector('[data-testid="page-back"]')).toBeNull();
    // Where it goes in the box; the approval mode and the model in the quiet line under it.
    const box = root.querySelector('[data-testid="home-composer"] .box');
    const status = root.querySelector('[data-testid="home-composer-status"]');
    expect(box?.querySelector('[data-testid="home-where"]')).not.toBeNull();
    expect(status?.querySelector('[data-testid="approval-mode"]')).not.toBeNull();
    expect(status?.querySelector('[data-testid="model-picker"]')).not.toBeNull();
    // Nothing to attach yet.
    expect(root.querySelector('[aria-label="Attach files"]')).toBeNull();
    await expectAccessible(root);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(
      fake.calls.map(({ method }) => method).filter((method) => method.endsWith('/start')),
    ).toEqual(['thread/start', 'turn/start']);
    expect(fake.paramsOf('thread/start')).toEqual([{ project: 'me' }]);
    expect(fake.paramsOf('turn/start')).toEqual([
      { thread: 't9', prompt: 'Plan the launch', _meta: { idempotency_key: KEY } },
    ]);
    // Nothing is carried to the thread's page: it shows what the server has.
    expect(navigate).toHaveBeenCalledWith(['/t', 't9']);
    navigate.mockRestore();
  });

  it('starts in the project the link names, on the model and in the mode picked', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake, 'p2');
    expect(where(root)).toBe('Where: Garden');
    // Opened for a project, the cursor is in the composer, to write the first message.
    expect(document.activeElement).toBe(root.querySelector('[data-testid="home-composer-input"]'));
    // A project's page has a way back.
    expect(root.querySelector('[data-testid="page-back"]')?.textContent.trim()).toBe('Back');
    root.querySelector<HTMLButtonElement>('[data-testid="model-picker-trigger"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    document.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[1]?.click();
    root.querySelector<HTMLButtonElement>('[data-testid="approval-mode-trigger"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    document.querySelectorAll<HTMLElement>('[data-testid="approval-mode-option"]')[0]?.click();
    await send(root, 'Plant tomatoes');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toEqual([
      { project: 'p2', model: 'hope-12b', approval: 'ask' },
    ]);
    navigate.mockRestore();
  });

  it('starts where the person picks: a chat, or a project', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake, 'me');
    expect(where(root)).toBe('Where: A chat');
    expect(root.querySelector('[data-testid="page-back"]')).toBeNull();
    root.querySelector<HTMLButtonElement>('[data-testid="home-where-trigger"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    const places = Array.from(
      document.querySelectorAll<HTMLElement>('[data-testid="home-where-option"]'),
    );
    expect(places.map((place) => place.textContent.trim())).toEqual(['A chat', 'glave', 'Garden']);
    places[1]?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(where(root)).toBe('Where: glave');
    await send(root, 'Fix the flaky test');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toEqual([{ project: 'p1' }]);
    navigate.mockRestore();
  });

  it('on a phone, puts where it goes behind the options button with the rest', async () => {
    const root = await render(session(), undefined, true);
    const status = root.querySelector('[data-testid="home-composer-status"]');
    expect(
      root.querySelector('[data-testid="home-composer"] .box [data-testid="home-where"]'),
    ).toBe(null);
    expect(status?.querySelector('[data-testid="home-where"]')).not.toBeNull();
    expect(where(root)).toBe('Where: A chat');
    await expectAccessible(root);
  });

  it('says what stays private only when glave runs on this computer', async () => {
    const root = await render(session());
    expect(root.querySelector('[data-testid="home-private"]')?.textContent.trim()).toBe(
      'Private on this computer. Only the model you pick sees what you send.',
    );
    TestBed.resetTestingModule();
    const fake = session();
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(fake, ELSEWHERE)],
    });
    const fixture = TestBed.createComponent(HomeComponent);
    await fixture.whenStable();
    expect(
      (fixture.nativeElement as HTMLElement).querySelector('[data-testid="home-private"]'),
    ).toBeNull();
  });

  it('holds the first message in the box until the server has it, then opens the thread', async () => {
    const turn = new FakeTurn();
    const fake = session().turn(turn);
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    await send(root, 'Plan the launch');
    const input = root.querySelector<HTMLTextAreaElement>('[data-testid="home-composer-input"]');
    // A second press, before the box shows it held, sends nothing more.
    input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
    await vi.waitFor(() => {
      expect([input?.value, input?.disabled]).toEqual(['Plan the launch', true]);
    });
    expect(navigate).not.toHaveBeenCalled();
    expect(
      [fake.paramsOf('thread/start'), fake.paramsOf('turn/start')].map((each) => each.length),
    ).toEqual([1, 1]);
    const started = event(1, 'turn.started', {});
    turn.send({ jsonrpc: '2.0', method: 'thread/event', params: { thread: 't9', event: started } });
    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith(['/t', 't9']);
    });
    navigate.mockRestore();
  });

  it('sends a first message whose answer was lost again as the same send', async () => {
    const fake = session().turn(new ApiError({ code: ErrorCode.INTERNAL, message: 'cut off' }));
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(root.querySelector('[data-testid="home-error"]')?.textContent).toContain('cut off');
    expect(navigate).not.toHaveBeenCalled();
    // Back in the box and sent as it was: to the thread made for it, with the same key, so the
    // server starts its turn once, though the first try's answer was lost.
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toHaveLength(1);
    const [first, again] = fake.paramsOf('turn/start');
    expect(again).toEqual(first);
    expect(navigate).toHaveBeenCalledWith(['/t', 't9']);
    navigate.mockRestore();
  });

  it('sends a first message the server refused, saying why, again as a new send', async () => {
    const full = new ApiError({ code: ErrorCode.QUEUE_FULL, message: 'The server is stopping' });
    const fake = session().turn(full);
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(root.querySelector('[data-testid="home-error"]')?.textContent).toContain('stopping');
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toHaveLength(2);
    const keys = fake.paramsOf('turn/start').map((params) => (params as TurnStartParams)._meta);
    expect(new Set(keys.map((meta) => meta?.idempotency_key)).size).toBe(2);
    navigate.mockRestore();
  });

  it('sends a changed first message as a new send, in a thread of its own', async () => {
    const fake = session().turn(new ApiError({ code: ErrorCode.INTERNAL, message: 'cut off' }));
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    await send(root, 'Plan the launch party');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toHaveLength(2);
    const keys = fake.paramsOf('turn/start').map((params) => (params as TurnStartParams)._meta);
    expect(new Set(keys.map((meta) => meta?.idempotency_key)).size).toBe(2);
    navigate.mockRestore();
  });

  it('keeps the draft and says why when the thread cannot start', async () => {
    const fake = session().answer(
      'thread/start',
      new ApiError({ code: ErrorCode.WORKSPACE, message: 'not a repository' }),
    );
    const root = await render(fake);
    await send(root, 'Hello');
    await new Promise((resolve) => setTimeout(resolve));
    expect(root.querySelector('[data-testid="home-error"]')?.textContent).toContain(
      'not a repository',
    );
    expect(
      root.querySelector<HTMLTextAreaElement>('[data-testid="home-composer-input"]')?.value,
    ).toBe('Hello');
  });
});

describe('HomeComponent with files', () => {
  it('keeps a file where the thread starts and sends it with the first message', async () => {
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: () => 'blob:1' });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: () => undefined });
    const sha = 'b'.repeat(64);
    const fake = session().answer('file/upload', {
      file: { sha256: sha, name: 'notes.txt', media_type: 'text/plain', bytes: 2 },
    });
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    const picker = root.querySelector<HTMLInputElement>('[data-testid="home-composer-picker"]');
    const file = new File(['hi'], 'notes.txt', { type: 'text/plain' });
    Object.defineProperty(picker, 'files', { configurable: true, value: [file] });
    picker?.dispatchEvent(new Event('change'));
    await vi.waitFor(() => {
      expect(fake.paramsOf('file/upload')).toEqual([
        { project: 'me', name: 'notes.txt', media_type: 'text/plain', data: 'aGk=' },
      ]);
    });
    await new Promise((resolve) => setTimeout(resolve));
    expect(root.querySelector('[data-testid="home-composer-file"]')?.textContent).toContain(
      'notes.txt',
    );
    await expectAccessible(root);
    await send(root, 'Summarize this');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('turn/start')).toEqual([
      {
        thread: 't9',
        prompt: 'Summarize this',
        files: [{ sha256: sha, name: 'notes.txt' }],
        _meta: { idempotency_key: KEY },
      },
    ]);
    expect(navigate).toHaveBeenCalledWith(['/t', 't9']);
    navigate.mockRestore();
  });
});
