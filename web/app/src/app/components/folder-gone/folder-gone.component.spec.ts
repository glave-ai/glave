import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  type ComponentFixture,
  DeferBlockBehavior,
  DeferBlockState,
  TestBed,
} from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ELSEWHERE, FakeSession, provideFakeSession } from '../../testing/fake-session';
import { FolderGoneComponent } from './folder-gone.component';

const GONE = 'This project’s folder isn’t there anymore.';

/** Lets the component's calls answer and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

function byTestId<T extends HTMLElement = HTMLElement>(root: HTMLElement, id: string): T | null {
  return root.querySelector<T>(`[data-testid="${id}"]`);
}

interface Rendered {
  readonly fixture: ComponentFixture<FolderGoneComponent>;
  readonly root: HTMLElement;
}

async function render(fake: FakeSession, local = true): Promise<Rendered> {
  const providers = provideFakeSession(fake, local ? undefined : ELSEWHERE);
  TestBed.configureTestingModule({
    providers: [
      ...providers,
      { provide: LiveAnnouncer, useValue: { announce: () => Promise.resolve() } },
    ],
  });
  const fixture = TestBed.createComponent(FolderGoneComponent);
  fixture.componentRef.setInput('testId', 'gone');
  fixture.componentRef.setInput('project', 'p1');
  fixture.componentRef.setInput('message', GONE);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

describe('FolderGoneComponent', () => {
  it('says what the server says, and offers to find the folder or remove the project', async () => {
    const { root } = await render(new FakeSession());
    expect(byTestId(root, 'gone-message')?.textContent.trim()).toBe(GONE);
    expect(byTestId(root, 'gone-find')?.textContent.trim()).toBe('Find it');
    expect(byTestId(root, 'gone-remove')?.textContent.trim()).toBe('Remove the project');
    expect(root.textContent).not.toMatch(/\/|-32029/u);
    await expectAccessible(root);
  });

  it('gives the project the folder picked as its new place', async () => {
    const fake = new FakeSession()
      .answer('folder/list', { path: '/Users/ana', parent: '/Users', folders: [] })
      .answer('project/list', { projects: [] });
    const { root, fixture } = await render(fake);
    byTestId<HTMLButtonElement>(root, 'gone-find')?.click();
    await settle(fixture);
    expect(byTestId(root, 'gone-picking')?.textContent).toContain(
      'Where is the project’s folder now?',
    );
    await expectAccessible(root);
    byTestId<HTMLButtonElement>(root, 'folder-picker-use')?.click();
    await settle(fixture);
    expect(fake.paramsOf('project/folder/set')).toEqual([{ project: 'p1', path: '/Users/ana' }]);
    expect(fake.paramsOf('project/list')).toHaveLength(1);
  });

  it('removes the project once the person confirms', async () => {
    const fake = new FakeSession().answer('project/list', { projects: [] });
    const { root, fixture } = await render(fake);
    byTestId<HTMLButtonElement>(root, 'gone-remove')?.click();
    await settle(fixture);
    expect(byTestId(root, 'gone-confirm')?.textContent).toContain('Its threads go to the archive');
    byTestId<HTMLButtonElement>(root, 'gone-confirm-cancel')?.click();
    await settle(fixture);
    expect(fake.paramsOf('project/remove')).toEqual([]);
    byTestId<HTMLButtonElement>(root, 'gone-remove')?.click();
    await settle(fixture);
    byTestId<HTMLButtonElement>(root, 'gone-confirm-ok')?.click();
    await settle(fixture);
    expect(fake.paramsOf('project/remove')).toEqual([{ project: 'p1' }]);
  });

  it('shows what the server refused, in its words', async () => {
    const fake = new FakeSession().answer(
      'project/remove',
      new ApiError({
        code: ErrorCode.NOT_ALLOWED,
        message: 'only the person on the computer glave runs on removes a project',
      }),
    );
    const { root, fixture } = await render(fake);
    byTestId<HTMLButtonElement>(root, 'gone-remove')?.click();
    await settle(fixture);
    byTestId<HTMLButtonElement>(root, 'gone-confirm-ok')?.click();
    await settle(fixture);
    expect(byTestId(root, 'gone-problem')?.textContent.trim()).toBe(
      'Only the person on the computer glave runs on removes a project.',
    );
    await expectAccessible(root);
  });

  it('says so when the folder browser cannot load its code', async () => {
    TestBed.configureTestingModule({ deferBlockBehavior: DeferBlockBehavior.Manual });
    const { root, fixture } = await render(new FakeSession());
    // Find it opened, as its code failed to load.
    const [picking] = await fixture.getDeferBlocks();
    await picking?.render(DeferBlockState.Error);
    expect(root.querySelector('[role="alert"]')?.textContent.trim()).toBe(
      'This part of glave could not load. Reload the page to try again.',
    );
    await expectAccessible(root);
  });

  it('only says so to someone signed in from elsewhere, who cannot reach the folders', async () => {
    const { root } = await render(new FakeSession(), false);
    expect(byTestId(root, 'gone-message')?.textContent.trim()).toBe(GONE);
    expect(byTestId(root, 'gone-find')).toBeNull();
    expect(byTestId(root, 'gone-remove')).toBeNull();
    await expectAccessible(root);
  });
});
