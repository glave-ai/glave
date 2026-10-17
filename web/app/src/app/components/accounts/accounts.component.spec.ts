import { TestBed } from '@angular/core/testing';
import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AccountsComponent } from './accounts.component';

const HOST: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000401',
  forge: 'gitea',
  url: 'https://code.example.com',
  login: 'ana',
  hint: 'wxyz',
};

const GITHUB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000402',
  forge: 'github',
  url: 'https://github.com',
  login: 'octocat',
  can: ['read', 'push', 'pull_requests'],
};

const INSTALL = 'https://github.com/apps/glave/installations/new';

/** Lets the component's promises and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** Types `value` into the field `testId`. */
function type(root: HTMLElement, testId: string, value: string): void {
  const input = root.querySelector<HTMLInputElement>(`[data-testid="${testId}"]`);
  if (input !== null) {
    input.value = value;
    input.dispatchEvent(new Event('input'));
  }
}

function click(root: ParentNode, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('AccountsComponent', () => {
  it('connects another host with its address and a token, and says why one was refused', async () => {
    let kept: readonly CredentialInfo[] = [];
    const fake = new FakeSession()
      .answer('credential/list', () => ({ credentials: kept, device: false }))
      .answer('credential/connect', (params) => {
        const { token } = params as { readonly token: string };
        if (token === 'wrong') {
          return new ApiError({
            code: ErrorCode.HOST,
            message: 'the git host at code.example.com did not accept that token',
          });
        }
        kept = [HOST];
        return { credential: HOST };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(AccountsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="accounts-none"]')).not.toBeNull();
    expect(root.textContent).not.toMatch(/config|policy|toml|--|token=/u);
    await expectAccessible(root);

    click(root, 'accounts-host-submit');
    await settle(fixture);
    expect(root.querySelector('[data-testid="accounts-host-problem"]')?.textContent).toContain(
      'Give the host’s address first.',
    );

    type(root, 'accounts-host-address', 'https://code.example.com');
    type(root, 'accounts-host-token', 'wrong');
    click(root, 'accounts-host-submit');
    await settle(fixture);
    expect(root.querySelector('[data-testid="accounts-host-problem"]')?.textContent).toContain(
      'The git host at code.example.com did not accept that token.',
    );

    type(root, 'accounts-host-token', 'gta_right');
    click(root, 'accounts-host-submit');
    await settle(fixture);
    expect(fake.paramsOf('credential/connect').at(-1)).toEqual({
      url: 'https://code.example.com',
      token: 'gta_right',
    });
    const row = root.querySelector('[data-testid="accounts-account"]');
    expect(row?.textContent).toContain('code.example.com (Gitea)');
    expect(row?.textContent).toContain('Signed in as ana, token ending wxyz');
    expect(row?.querySelector('[data-testid="accounts-abilities"]')).toBeNull();
    expect(row?.querySelector('[data-testid="accounts-abilities-note"]')?.textContent).toContain(
      'depends on the access you gave its token',
    );
    expect(root.querySelector('[data-testid="accounts-host-problem"]')).toBeNull();
    await expectAccessible(root);
  });

  it('connects GitHub with a token when its sign-in in the browser is not set up', async () => {
    const fake = new FakeSession()
      .answer('credential/list', { credentials: [], device: false })
      .answer('credential/connect', { credential: GITHUB });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(AccountsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="accounts-github-sign-in"]')).toBeNull();
    type(root, 'accounts-github-token', 'test-token');
    click(root, 'accounts-github-submit');
    await settle(fixture);
    expect(fake.paramsOf('credential/connect')).toEqual([
      { url: 'https://github.com', token: 'test-token' },
    ]);
  });

  it('signs in to GitHub in the browser: shows the code and link, then the account', async () => {
    let approved = false;
    let polls = 0;
    const fake = new FakeSession()
      .answer('credential/list', () => ({
        credentials: approved ? [GITHUB] : [],
        device: true,
        install: INSTALL,
      }))
      .answer('credential/device/start', {
        device: {
          flow: 'f1',
          code: 'WDJB-MJHT',
          url: 'https://github.com/login/device',
          expires_in: 900,
        },
      })
      .answer('credential/device/poll', async () => {
        polls += 1;
        await new Promise((resolve) => setTimeout(resolve));
        if (polls < 3) {
          return { flow: 'f1' };
        }
        approved = true;
        return { credential: GITHUB };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(AccountsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    expect(
      root.querySelector('[data-testid="accounts-github-repositories"]')?.getAttribute('href'),
    ).toBe(INSTALL);
    click(root, 'accounts-github-sign-in');
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="accounts-device-code"]')?.textContent).toBe(
      'WDJB-MJHT',
    );
    expect(root.querySelector('[data-testid="accounts-device-open"]')?.getAttribute('href')).toBe(
      'https://github.com/login/device',
    );
    await expectAccessible(root);
    await settle(fixture);
    await settle(fixture);
    expect(root.querySelector('[data-testid="accounts-device"]')).toBeNull();
    expect(root.querySelector('[data-testid="accounts-account"]')?.textContent).toContain('GitHub');
    const abilities = root.querySelectorAll('[data-testid="accounts-abilities"] li');
    expect(Array.from(abilities, (item) => item.textContent.trim())).toEqual([
      'Read and clone your repositories',
      'Push branches',
      'Open and comment on pull requests',
    ]);
    await expectAccessible(root);
  });

  it('removes an account once confirmed', async () => {
    let kept: readonly CredentialInfo[] = [HOST];
    const fake = new FakeSession()
      .answer('credential/list', () => ({ credentials: kept, device: false }))
      .answer('credential/remove', () => {
        kept = [];
        return {};
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(AccountsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    click(root, 'accounts-remove');
    await fixture.whenStable();
    click(document, 'accounts-remove-confirm-ok');
    await settle(fixture);
    expect(fake.paramsOf('credential/remove')).toEqual([{ credential: HOST.id }]);
    expect(root.querySelector('[data-testid="accounts-account"]')).toBeNull();
  });
});
