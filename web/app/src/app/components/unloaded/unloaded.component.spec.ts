import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { RELOAD, UnloadedComponent } from './unloaded.component';

/**
 * The notice, in a page that loaded from a glave of `SERVER`'s version, once the server is
 * asked again and answers `now`: its version then, or a failure to reach it.
 */
async function render(now: string | ApiError): Promise<{
  readonly root: HTMLElement;
  readonly reloads: () => number;
}> {
  let version: string | ApiError = SERVER.version;
  const fake = new FakeSession().answer('server/discover', () =>
    version instanceof ApiError ? version : { ...SERVER, version },
  );
  let reloaded = 0;
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      {
        provide: RELOAD,
        useValue: () => {
          reloaded += 1;
        },
      },
    ],
  });
  await TestBed.inject(SessionService).start();
  version = now;
  const fixture = TestBed.createComponent(UnloadedComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, reloads: () => reloaded };
}

function sentence(root: HTMLElement): string | undefined {
  return root.querySelector('[role="alert"]')?.textContent.trim();
}

describe('UnloadedComponent', () => {
  it('says the part could not load and offers Reload, which loads the page again', async () => {
    const { root, reloads } = await render(SERVER.version);
    expect(sentence(root)).toBe('This part of glave could not load.');
    await expectAccessible(root);
    const reload = root.querySelector<HTMLButtonElement>('[data-testid="unloaded-reload"]');
    expect(reload?.textContent.trim()).toBe('Reload');
    reload?.click();
    expect(reloads()).toBe(1);
  });

  it('says glave was updated when the server now says another version', async () => {
    const { root } = await render('0.2.0');
    expect(sentence(root)).toBe('glave was updated. Reload to continue.');
    await expectAccessible(root);
  });

  it('says only that it could not load when the server is out of reach', async () => {
    const away = new ApiError({ code: ErrorCode.INTERNAL, message: 'unreachable' });
    const { root } = await render(away);
    expect(sentence(root)).toBe('This part of glave could not load.');
  });
});
