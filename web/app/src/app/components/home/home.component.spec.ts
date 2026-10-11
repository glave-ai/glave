import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space, thread } from '../../testing/fixtures';
import { HomeComponent } from './home.component';

async function send(root: HTMLElement, text: string): Promise<void> {
  const input = root.querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]');
  if (input) {
    input.value = text;
    input.dispatchEvent(new Event('input'));
  }
  await Promise.resolve();
  input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
}

describe('HomeComponent', () => {
  it('starts a thread in the first space and opens it with the prompt in navigation state', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [space('p1', 'glave')] })
      .answer('thread/list', { threads: [] })
      .answer('thread/start', { thread: thread('t9', null) });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    await TestBed.inject(SpacesService).refresh();
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true);
    const fixture = TestBed.createComponent(HomeComponent);
    await fixture.whenStable();
    await expectAccessible(fixture.nativeElement);
    await send(fixture.nativeElement, 'Plan the launch');
    await fixture.whenStable();
    expect(fake.paramsOf('thread/start')).toEqual([{ space: 'p1' }]);
    expect(navigate).toHaveBeenCalledWith(['/t', 't9'], { state: { prompt: 'Plan the launch' } });
  });

  it('keeps the draft and says why when the thread cannot start', async () => {
    const fake = new FakeSession().answer(
      'thread/start',
      new ApiError({ code: ErrorCode.WORKSPACE, message: 'not a repository' }),
    );
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(HomeComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    await send(root, 'Hello');
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="home-error"]')?.textContent).toContain(
      'not a repository',
    );
    expect(root.querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')?.value).toBe(
      'Hello',
    );
  });
});
