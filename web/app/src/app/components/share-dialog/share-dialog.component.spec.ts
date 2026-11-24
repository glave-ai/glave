import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type ShareInfo, type ShareOf } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ShareDialogComponent } from './share-dialog.component';

/** A file a turn made, at the version its card shows. */
const BUDGET: ShareOf = { kind: 'file', thread: 't1', sha256: 'b'.repeat(64) };

/** The budget shared with anyone, at the card's version. */
const SHARED: ShareInfo = {
  audience: { kind: 'anyone' },
  link: 'https://abcdefghijklmnopqrstuvwxyz.share.example.net/',
  shared_at: '2026-11-24T10:00:00.000Z',
  sha256: 'b'.repeat(64),
  said: 'Shared with anyone who has the link',
};

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function byTestId<T extends HTMLElement>(testId: string): T | null {
  return document.querySelector<T>(`[data-testid="${testId}"]`);
}

function choose(value: string): void {
  document
    .querySelector<HTMLInputElement>(`[data-testid="share-who"] input[value="${value}"]`)
    ?.click();
}

async function opened(
  fake: FakeSession,
  of: ShareOf = BUDGET,
): Promise<{ fixture: ComponentFixture<ShareDialogComponent>; changes: (ShareInfo | null)[] }> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(ShareDialogComponent);
  const changes: (ShareInfo | null)[] = [];
  fixture.componentInstance.changed.subscribe((share) => changes.push(share));
  fixture.componentRef.setInput('of', of);
  fixture.componentRef.setInput('name', 'Trip budget');
  fixture.componentRef.setInput('open', true);
  await settle(fixture);
  return { fixture, changes };
}

describe('ShareDialogComponent', () => {
  it('shares a file with anyone, then shows its link to copy', async () => {
    const fake = new FakeSession()
      .answer('share/get', {})
      .answer('share/set', { share: SHARED });
    const { fixture, changes } = await opened(fake);
    await expectAccessible(document.body);
    expect(byTestId('share-link')).toBeNull();
    expect(document.body.textContent).toContain('Share “Trip budget”');
    choose('anyone');
    await settle(fixture);
    byTestId('share-apply')?.click();
    await settle(fixture);
    expect(fake.paramsOf('share/set')).toEqual([
      { ...BUDGET, audience: { kind: 'anyone' }, update: false },
    ]);
    expect(byTestId<HTMLInputElement>('share-link')?.value).toBe(SHARED.link);
    expect(byTestId('share-copy')).not.toBeNull();
    expect(changes).toEqual([SHARED]);
    await expectAccessible(document.body);
  });

  it('stops sharing with Only you, and says the link stops working', async () => {
    const fake = new FakeSession().answer('share/get', { share: SHARED }).answer('share/set', {});
    const { fixture, changes } = await opened(fake);
    expect(document.body.textContent).toContain('The link stops working');
    choose('only_you');
    await settle(fixture);
    byTestId('share-apply')?.click();
    await settle(fixture);
    expect(fake.paramsOf('share/set')).toEqual([BUDGET]);
    expect(byTestId('share-link')).toBeNull();
    expect(changes).toEqual([null]);
  });

  it('offers this version where the file changed since it was shared, the same link', async () => {
    const older = { ...SHARED, sha256: 'a'.repeat(64) };
    const fake = new FakeSession()
      .answer('share/get', { share: older })
      .answer('share/set', { share: SHARED });
    const { fixture } = await opened(fake);
    expect(byTestId('share-older')).not.toBeNull();
    await expectAccessible(document.body);
    byTestId('share-update')?.click();
    await settle(fixture);
    expect(fake.paramsOf('share/set')).toEqual([
      { ...BUDGET, audience: { kind: 'anyone' }, update: true },
    ]);
    expect(byTestId('share-older')).toBeNull();
  });

  it('never offers a Library item a newer version', async () => {
    const item: ShareOf = { kind: 'item', project: 'p1', asset: 'a1' };
    const fake = new FakeSession().answer('share/get', { share: SHARED });
    await opened(fake, item);
    expect(fake.paramsOf('share/get')).toEqual([item]);
    expect(byTestId('share-older')).toBeNull();
  });

  it('says why it could not share, in the server’s words', async () => {
    const refused = new ApiError({
      code: ErrorCode.NOT_AVAILABLE,
      message: "This server doesn't share files",
    });
    const fake = new FakeSession().answer('share/get', {}).answer('share/set', refused);
    const { fixture } = await opened(fake);
    choose('anyone');
    await settle(fixture);
    byTestId('share-apply')?.click();
    await settle(fixture);
    expect(byTestId('share-problem')?.textContent).toContain("This server doesn't share files.");
  });
});
