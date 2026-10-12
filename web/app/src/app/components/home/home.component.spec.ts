import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

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

  it('starts a thread in the first space and opens it with the prompt in navigation state', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake);
    expect(root.querySelector('[data-testid="approval-mode"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="model-picker-trigger"]')?.textContent).toContain(
      'GPT-6 Luna',
    );
    expect(root.querySelector('[data-testid="home-space"]')).toBeNull();
    await expectAccessible(root);
    await send(root, 'Plan the launch');
    await new Promise((resolve) => setTimeout(resolve));
    expect(fake.paramsOf('thread/start')).toEqual([{ space: 'p1' }]);
    expect(navigate).toHaveBeenCalledWith(['/t', 't9'], { state: { prompt: 'Plan the launch' } });
    navigate.mockRestore();
  });

  it('starts in the space the link names, on the model and in the mode picked', async () => {
    const fake = session();
    const navigate = vi.spyOn(Router.prototype, 'navigate').mockResolvedValue(true);
    const root = await render(fake, 'p2');
    expect(root.querySelector('[data-testid="home-space"]')?.textContent).toBe('In Garden');
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
