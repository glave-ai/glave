import { TestBed } from '@angular/core/testing';
import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { CredentialsService } from '../../services/credentials.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { GithubRowComponent } from './github-row.component';

const GITHUB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000402',
  forge: 'github',
  url: 'https://github.com',
  login: 'octocat',
  can: ['read', 'push', 'pull_requests'],
};

const GITLAB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000403',
  forge: 'gitlab',
  url: 'https://gitlab.com',
  login: 'ana',
};

/** Where the server says GitHub's sign-in goes. */
const GITHUB_SIGN_IN = 'https://github.com';

const INSTALL = 'https://github.com/apps/glave/installations/new';

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function byTestId(root: ParentNode, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

/** The row, its accounts listed first, as the Connections page lists them. */
async function rendered(fake: FakeSession): Promise<{
  readonly fixture: { whenStable(): Promise<unknown>; detectChanges(): void };
  readonly root: HTMLElement;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  await TestBed.inject(CredentialsService).load();
  const fixture = TestBed.createComponent(GithubRowComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

describe('GithubRowComponent', () => {
  it('signs in on GitHub: its code, copied as GitHub opens, then whom it is connected as', async () => {
    let kept: readonly CredentialInfo[] = [];
    let polls = 0;
    const fake = new FakeSession()
      .answer('credential/list', () => ({
        credentials: kept,
        install: INSTALL,
        github: GITHUB_SIGN_IN,
      }))
      .answer('credential/device/start', {
        device: { flow: 'f1', code: 'WDJB-MJHT', url: 'https://github.com/login/device' },
      })
      .answer('credential/device/poll', () => {
        polls += 1;
        if (polls < 2) {
          return { flow: 'f1' };
        }
        kept = [GITHUB];
        return { credential: GITHUB };
      });
    const { fixture, root } = await rendered(fake);
    expect(byTestId(root, 'github-row')?.textContent).toContain(
      'Let glave work in your repositories',
    );
    await expectAccessible(root);

    byTestId(root, 'github-connect')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    fixture.detectChanges();
    await settle(fixture);
    expect(byTestId(root, 'github-connected')?.textContent).toContain(
      'Connected as octocat. glave can clone, push and open pull requests',
    );
    expect(byTestId(root, 'github-repositories')?.getAttribute('href')).toBe(INSTALL);
    expect(root.textContent).not.toMatch(/token ending/u);
    await expectAccessible(root);
  });

  it('shows the code to enter while it waits, and Cancel stops waiting', async () => {
    const fake = new FakeSession()
      .answer('credential/list', { credentials: [], github: GITHUB_SIGN_IN })
      .answer('credential/device/start', {
        device: { flow: 'f1', code: 'WDJB-MJHT', url: 'https://github.com/login/device' },
      })
      .answer('credential/device/poll', () => new Promise(() => undefined));
    const { fixture, root } = await rendered(fake);
    byTestId(root, 'github-connect')?.click();
    await settle(fixture);
    expect(byTestId(root, 'github-code')?.textContent).toContain('WDJB-MJHT');
    expect(byTestId(root, 'github-open')?.getAttribute('href')).toBe(
      'https://github.com/login/device',
    );
    expect(byTestId(root, 'github-signing')?.textContent).toContain(
      'Waiting while you approve glave on GitHub.',
    );
    await expectAccessible(root);
    byTestId(root, 'github-cancel')?.click();
    await settle(fixture);
    expect(byTestId(root, 'github-signing')).toBeNull();
    expect(byTestId(root, 'github-connect')).not.toBeNull();
  });

  it('lists another git host, and disconnects one once asked', async () => {
    let kept: readonly CredentialInfo[] = [GITHUB, GITLAB];
    const fake = new FakeSession()
      .answer('credential/list', () => ({ credentials: kept, github: GITHUB_SIGN_IN }))
      .answer('credential/remove', () => {
        kept = [GITHUB];
        return {};
      });
    const { fixture, root } = await rendered(fake);
    expect(byTestId(root, 'git-host')?.textContent).toContain('gitlab.com (GitLab)');
    expect(byTestId(root, 'git-host')?.textContent).toContain('Signed in as ana');
    byTestId(root, 'git-host-disconnect')?.click();
    await settle(fixture);
    byTestId(document, 'github-disconnect-confirm-ok')?.click();
    await settle(fixture);
    expect(fake.paramsOf('credential/remove')).toEqual([{ credential: GITLAB.id }]);
    expect(byTestId(root, 'git-host')).toBeNull();
  });

  it('disconnects GitHub itself once asked, and offers Connect again', async () => {
    let kept: readonly CredentialInfo[] = [GITHUB];
    const fake = new FakeSession()
      .answer('credential/list', () => ({ credentials: kept, github: GITHUB_SIGN_IN }))
      .answer('credential/remove', () => {
        kept = [];
        return {};
      });
    const { fixture, root } = await rendered(fake);
    byTestId(root, 'github-disconnect')?.click();
    await settle(fixture);
    byTestId(document, 'github-disconnect-confirm-ok')?.click();
    await settle(fixture);
    expect(fake.paramsOf('credential/remove')).toEqual([{ credential: GITHUB.id }]);
    expect(byTestId(root, 'github-connect')).not.toBeNull();
  });

  it('says why GitHub would not sign in, and offers Connect again', async () => {
    const refused = new ApiError({
      code: ErrorCode.NOT_ALLOWED,
      message: 'GitHub said the code expired: connect again',
    });
    const fake = new FakeSession()
      .answer('credential/list', { credentials: [], github: GITHUB_SIGN_IN })
      .answer('credential/device/start', {
        device: { flow: 'f1', code: 'WDJB-MJHT', url: 'https://github.com/login/device' },
      })
      .answer('credential/device/poll', refused);
    const { fixture, root } = await rendered(fake);
    byTestId(root, 'github-connect')?.click();
    await settle(fixture);
    expect(byTestId(root, 'github-problem')?.textContent).toContain('the code expired');
    expect(byTestId(root, 'github-signing')).toBeNull();
    expect(byTestId(root, 'github-connect')).not.toBeNull();
    await expectAccessible(root);
  });

  it('claims only what GitHub gave: nothing for a token, a choice for no repositories yet', async () => {
    for (const [can, said] of [
      [undefined, 'Connected as octocat.'],
      [[], 'Connected as octocat. Now choose which repositories glave may use.'],
      [['read'], 'Connected as octocat.'],
    ] as const) {
      TestBed.resetTestingModule();
      const github: CredentialInfo = { ...GITHUB, can };
      const fake = new FakeSession().answer('credential/list', {
        credentials: [github],
        github: GITHUB_SIGN_IN,
      });
      const { root } = await rendered(fake);
      expect(byTestId(root, 'github-connected')?.textContent.trim()).toBe(said);
    }
  });

  it('is the GitHub Enterprise Server the server signs in at, not github.com alone', async () => {
    const enterprise: CredentialInfo = { ...GITHUB, url: 'https://github.example.com' };
    const fake = new FakeSession().answer('credential/list', {
      credentials: [enterprise],
      github: 'https://github.example.com',
    });
    const { root } = await rendered(fake);
    expect(byTestId(root, 'github-connected')?.textContent).toContain('Connected as octocat');
    expect(byTestId(root, 'git-host')).toBeNull();
  });
});
