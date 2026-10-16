import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { TOKEN_KEY } from '../../lib/token';
import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS, SERVER, space, thread } from '../../testing/fixtures';
import { HomeComponent } from './home.component';

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
    .answer('space/list', {
      spaces: [space('p1', 'glave'), space('p2', 'Garden'), space('me', 'Personal', 'personal')],
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

async function render(fake: FakeSession, spaceId?: string): Promise<HTMLElement> {
  TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
  await TestBed.inject(SpacesService).refresh();
  const fixture = TestBed.createComponent(HomeComponent);
  if (spaceId !== undefined) {
    fixture.componentRef.setInput('space', spaceId);
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

  it('starts a thread in the personal space and opens it with the prompt in navigation state', async () => {
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
    // Nothing to attach yet.
    expect(root.querySelector('[aria-label="Attach files"]')).toBeNull();
    await expectAccessible(root);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toEqual([{ space: 'me' }]);
    expect(navigate).toHaveBeenCalledWith(['/t', 't9'], { state: { prompt: 'Plan the launch' } });
    navigate.mockRestore();
  });

  it('starts in the space the link names, on the model and in the mode picked', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake, 'p2');
    expect(where(root)).toBe('Where: Garden');
    root.querySelector<HTMLButtonElement>('[data-testid="model-picker-trigger"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    document.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[1]?.click();
    root.querySelector<HTMLButtonElement>('[data-testid="approval-mode-trigger"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    document.querySelectorAll<HTMLElement>('[data-testid="approval-mode-option"]')[0]?.click();
    await send(root, 'Plant tomatoes');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toEqual([
      { space: 'p2', model: 'hope-12b', approval: 'ask' },
    ]);
    navigate.mockRestore();
  });

  it('starts where the person picks: a chat, or a project', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake, 'me');
    expect(where(root)).toBe('Where: A chat');
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
    expect(fake.paramsOf('thread/start')).toEqual([{ space: 'p1' }]);
    navigate.mockRestore();
  });

  it('says what stays private only when glave runs on this computer', async () => {
    const root = await render(session());
    expect(root.querySelector('[data-testid="home-private"]')?.textContent.trim()).toBe(
      'Private on this computer. Only the model you pick sees what you send.',
    );
    TestBed.resetTestingModule();
    const fake = session();
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(fake)],
    });
    sessionStorage.setItem(TOKEN_KEY, '"an-access-token"');
    const fixture = TestBed.createComponent(HomeComponent);
    await fixture.whenStable();
    expect(
      (fixture.nativeElement as HTMLElement).querySelector('[data-testid="home-private"]'),
    ).toBeNull();
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
