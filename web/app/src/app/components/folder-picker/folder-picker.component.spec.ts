import { LiveAnnouncer } from '@angular/cdk/a11y';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import type { Mock } from 'vitest';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { FolderPickerComponent } from './folder-picker.component';

/** Lets the component's calls answer and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

/** The folders on the fake computer, by path; the home folder is /Users/ana. */
const TREE: Readonly<Record<string, readonly string[]>> = {
  '/': ['Users'],
  '/Users': ['ana'],
  '/Users/ana': ['code', 'Documents'],
  '/Users/ana/code': ['app', 'notes'],
  '/Users/ana/code/app': [],
  '/Users/ana/Documents': [],
};

function listed(params: unknown): unknown {
  const path = (params as { path?: string | null }).path ?? '/Users/ana';
  const folders = TREE[path];
  if (folders === undefined) {
    return new ApiError({
      code: ErrorCode.INVALID_PARAMS,
      message: `there is no folder at ${path}`,
    });
  }
  const parent = path === '/' ? null : path.slice(0, path.lastIndexOf('/')) || '/';
  return { path, parent, folders, ...refusing(path) };
}

/** What the server says of the home folder, in its own words. */
const HOME_REFUSED =
  "your home folder holds your Chats, so it is no project's folder: pick a folder inside it";

/** What the server says of a folder that is never a project's: the home folder and above. */
function refusing(path: string): { readonly refused?: string } {
  if (path === '/Users/ana') {
    return { refused: HOME_REFUSED };
  }
  return '/Users/ana'.startsWith(path) ? { refused: 'this folder holds your home folder' } : {};
}

/** The folders on a fake Windows computer, by path, under its drive C:. */
const WINDOWS: Readonly<Record<string, readonly string[]>> = {
  'C:\\': ['Users'],
  'C:\\Users\\ana': ['code'],
  'C:\\Users\\ana\\code': [],
};

function listedOnWindows(params: unknown): unknown {
  const path = (params as { path?: string | null }).path ?? 'C:\\Users\\ana';
  return { path, parent: null, folders: WINDOWS[path] ?? [] };
}

/** The picker as a test sees it: what it asked, said and picked. */
interface Rendered {
  readonly fixture: ComponentFixture<FolderPickerComponent>;
  readonly fake: FakeSession;
  readonly announce: Mock<(message: string) => Promise<void>>;
  readonly picked: readonly string[];
  readonly root: HTMLElement;
}

async function render(start: string | null = null, answer = listed): Promise<Rendered> {
  const fake = new FakeSession().answer('folder/list', answer);
  const announce = vi.fn<(message: string) => Promise<void>>(() => Promise.resolve());
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), { provide: LiveAnnouncer, useValue: { announce } }],
  });
  const fixture = TestBed.createComponent(FolderPickerComponent);
  fixture.componentRef.setInput('start', start);
  const picked: string[] = [];
  fixture.componentInstance.picked.subscribe((path) => picked.push(path));
  await settle(fixture);
  return { fixture, fake, announce, picked, root: fixture.nativeElement as HTMLElement };
}

function all(root: HTMLElement, testId: string): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>(`[data-testid="${testId}"]`));
}

function texts(root: HTMLElement, testId: string): readonly string[] {
  return all(root, testId).map((each) => each.textContent.trim());
}

describe('FolderPickerComponent', () => {
  it('opens at the home folder: where it is from the top, and the folders in it', async () => {
    const { root, fake } = await render();
    expect(fake.paramsOf('folder/list')).toEqual([{}]);
    const top = root.querySelector('[data-testid="folder-picker-top"]');
    expect(top?.getAttribute('aria-label')).toBe('This computer');
    expect(texts(root, 'folder-picker-crumb')).toEqual(['Users', 'ana']);
    expect(all(root, 'folder-picker-crumb').at(-1)?.getAttribute('aria-current')).toBe('location');
    expect(top?.hasAttribute('aria-current')).toBe(false);
    expect(texts(root, 'folder-picker-folder')).toEqual(['code', 'Documents']);
    expect(
      root.querySelector('[data-testid="folder-picker-folders"]')?.getAttribute('aria-label'),
    ).toBe('Folders in ana');
    await expectAccessible(root);
  });

  it('goes into a folder, says where it is, and moves focus into it', async () => {
    const { root, fixture, fake, announce } = await render();
    all(root, 'folder-picker-folder')[0]?.click();
    await settle(fixture);
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: '/Users/ana/code' });
    expect(texts(root, 'folder-picker-crumb')).toEqual(['Users', 'ana', 'code']);
    expect(texts(root, 'folder-picker-folder')).toEqual(['app', 'notes']);
    expect(announce).toHaveBeenCalledWith('code, 2 folders');
    expect(document.activeElement).toBe(all(root, 'folder-picker-folder')[0]);
    await expectAccessible(root);

    // A folder with none in it says so, and focus moves to Use this folder.
    all(root, 'folder-picker-folder')[0]?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="folder-picker-empty"]')?.textContent).toBe(
      'No folders in here.',
    );
    expect(announce).toHaveBeenLastCalledWith('app, no folders');
    expect(document.activeElement).toBe(root.querySelector('[data-testid="folder-picker-use"]'));
    await expectAccessible(root);
  });

  it('goes up from the path, keeping focus on the folder pressed, up to the top', async () => {
    const { root, fixture, fake, announce } = await render();
    const users = all(root, 'folder-picker-crumb')[0];
    users?.focus();
    users?.click();
    await settle(fixture);
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: '/Users' });
    expect(texts(root, 'folder-picker-crumb')).toEqual(['Users']);
    expect(all(root, 'folder-picker-crumb')[0]).toBe(users);
    expect(document.activeElement).toBe(users);
    root.querySelector<HTMLButtonElement>('[data-testid="folder-picker-top"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: '/' });
    expect(texts(root, 'folder-picker-crumb')).toEqual([]);
    expect(texts(root, 'folder-picker-folder')).toEqual(['Users']);
    expect(announce).toHaveBeenLastCalledWith(
      'This computer, 1 folder. This folder holds your home folder.',
    );
  });

  it('picks the folder shown', async () => {
    const { root, fixture, picked } = await render('/Users/ana/code');
    expect(root.querySelector('[data-testid="folder-picker-refused"]')).toBeNull();
    root.querySelector<HTMLButtonElement>('[data-testid="folder-picker-use"]')?.click();
    await settle(fixture);
    expect(picked).toEqual(['/Users/ana/code']);
  });

  it('cannot pick a folder the server says is none, says why, and goes into it', async () => {
    const { root, fixture, picked } = await render();
    const use = root.querySelector<HTMLButtonElement>('[data-testid="folder-picker-use"]');
    expect(use?.disabled).toBe(true);
    const refused = root.querySelector('[data-testid="folder-picker-refused"]');
    expect(refused?.textContent.trim()).toBe(
      "Your home folder holds your Chats, so it is no project's folder: pick a folder inside it.",
    );
    expect(use?.getAttribute('aria-describedby')).toBe(refused?.id);
    await expectAccessible(root);
    use?.click();
    all(root, 'folder-picker-folder')[0]?.click();
    await settle(fixture);
    expect(use?.disabled).toBe(false);
    expect(use?.hasAttribute('aria-describedby')).toBe(false);
    expect(root.querySelector('[data-testid="folder-picker-refused"]')).toBeNull();
    use?.click();
    expect(picked).toEqual(['/Users/ana/code']);
  });

  it('keeps the folder shown when another cannot be listed, and says why', async () => {
    const { root, fixture, fake } = await render();
    fake.answer('folder/list', (params) =>
      (params as { path?: string }).path === '/Users/ana/Documents'
        ? new ApiError({
            code: ErrorCode.INVALID_PARAMS,
            message: 'permission denied: /Users/ana/Documents',
          })
        : listed(params),
    );
    all(root, 'folder-picker-folder')[1]?.click();
    await settle(fixture);
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      'Permission denied: /Users/ana/Documents.',
    );
    expect(texts(root, 'folder-picker-folder')).toEqual(['code', 'Documents']);
    all(root, 'folder-picker-folder')[0]?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="folder-picker-problem"]')).toBeNull();
  });

  it('on Windows, starts at the drive’s root and joins folders with backslashes', async () => {
    const { root, fixture, fake } = await render(null, listedOnWindows);
    expect(root.querySelector('[data-testid="folder-picker-top"]')).toBeNull();
    expect(texts(root, 'folder-picker-crumb')).toEqual(['C:', 'Users', 'ana']);
    await expectAccessible(root);
    all(root, 'folder-picker-folder')[0]?.click();
    await settle(fixture);
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: 'C:\\Users\\ana\\code' });
    all(root, 'folder-picker-crumb')[0]?.click();
    await settle(fixture);
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: 'C:\\' });
    expect(texts(root, 'folder-picker-crumb')).toEqual(['C:']);
    expect(
      root.querySelector('[data-testid="folder-picker-folders"]')?.getAttribute('aria-label'),
    ).toBe('Folders in C:');
  });
});
