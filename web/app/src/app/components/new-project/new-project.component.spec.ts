import { LiveAnnouncer } from '@angular/cdk/a11y';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, type CredentialInfo, ErrorCode, type Project } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import type { Mock } from 'vitest';

import { ProjectsService } from '../../services/projects.service';
import { ELSEWHERE, FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record } from '../../testing/fixtures';
import { NewProjectComponent } from './new-project.component';

/** Lets the component's calls answer and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

const GITHUB: CredentialInfo = { id: 'c1', forge: 'github', url: 'https://github.com' };

interface Options {
  /** Whether the session is with a glave on this computer. */
  readonly local?: boolean;
  readonly accounts?: readonly CredentialInfo[];
  /** How `project/create` answers; by default it makes the project asked for, as `p9`. */
  readonly create?: (params: unknown) => unknown;
}

/** The dialog as a test sees it: what it asked and said, and how often it navigated. */
interface Rendered {
  readonly fixture: ComponentFixture<NewProjectComponent>;
  readonly fake: FakeSession;
  readonly root: HTMLElement;
  readonly announce: Mock<(message: string) => Promise<void>>;
  readonly navigated: () => number;
}

async function render(options: Options = {}): Promise<Rendered> {
  const made: Project[] = [];
  const fake = new FakeSession()
    .answer('folder/list', (params) => {
      const path = (params as { path?: string }).path ?? '/Users/ana';
      const folders: Readonly<Record<string, readonly string[]>> = {
        '/Users/ana': ['code'],
        '/Users/ana/code': ['app'],
        '/Users/ana/code/app': [],
      };
      return { path, parent: '/Users', folders: folders[path] ?? [] };
    })
    .answer('credential/list', { credentials: options.accounts ?? [], device: false })
    .answer('project/list', () => ({ projects: made }))
    .answer('thread/list', { threads: [] })
    .answer(
      'project/create',
      options.create ??
        ((params) => {
          const project = record('p9', (params as { name: string }).name);
          made.push(project);
          return { project: project };
        }),
    );
  const announce = vi.fn<(message: string) => Promise<void>>(() => Promise.resolve());
  const providers = provideFakeSession(fake, options.local === false ? ELSEWHERE : undefined);
  TestBed.configureTestingModule({
    providers: [
      provideRouter([{ path: '**', children: [] }]),
      ...providers,
      { provide: LiveAnnouncer, useValue: { announce } },
    ],
  });
  const fixture = TestBed.createComponent(NewProjectComponent);
  let navigated = 0;
  fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
  fixture.componentRef.setInput('open', true);
  await settle(fixture);
  const root = fixture.nativeElement as HTMLElement;
  return { fixture, fake, root, announce, navigated: () => navigated };
}

function find<T extends Element = HTMLElement>(root: Element, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

function type(root: Element, testId: string, value: string): void {
  const input = find<HTMLInputElement>(root, testId);
  input.value = value;
  input.dispatchEvent(new Event('input'));
}

function choose(root: Element, label: string): void {
  const radios = Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
  radios.find((radio) => radio.closest('label')?.textContent.startsWith(label))?.click();
}

function create(root: Element): HTMLButtonElement {
  return find<HTMLButtonElement>(root, 'new-project-create');
}

describe('NewProjectComponent', () => {
  it('starts from a folder on this computer, picked from the home folder', async () => {
    const { root } = await render();
    expect(find(root, 'new-project').querySelector('h2')?.textContent).toBe('New project');
    const radios = Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
    const told = (radio: HTMLInputElement, by: string): string | undefined =>
      root.querySelector(`[id="${radio.getAttribute(by) ?? ''}"]`)?.textContent.trim();
    expect(
      radios.map((radio) => [told(radio, 'aria-labelledby'), told(radio, 'aria-describedby')]),
    ).toEqual([
      ['A folder on this computer', 'Work on files you already have.'],
      ['A repository', 'Clone one from GitHub or another git host.'],
      ['Empty', 'glave makes a new folder for it.'],
    ]);
    expect(radios[0]?.checked).toBe(true);
    expect(find(root, 'new-project-folder').textContent).toContain('Folder');
    expect(root.querySelector('app-folder-picker')).not.toBeNull();
    expect(create(root).disabled).toBe(true);
    expect(root.textContent).not.toMatch(/space|workspace/iu);
    await expectAccessible(root);
  });

  it('makes a project of the folder picked, named after it, and goes into it', async () => {
    const { root, fixture, fake, announce, navigated } = await render();
    find(root, 'folder-picker-folder').click();
    await settle(fixture);
    find(root, 'folder-picker-folder').click();
    await settle(fixture);
    find(root, 'folder-picker-use').click();
    await settle(fixture);
    expect(find(root, 'new-project-folder-path').textContent).toBe('/Users/ana/code/app');
    const name = find<HTMLInputElement>(root, 'new-project-name');
    // The folder's name, as a value the person can change, not a placeholder.
    expect(name.value).toBe('app');
    expect(name.placeholder).toBe('');
    await expectAccessible(root);
    create(root).click();
    await settle(fixture);
    expect(fake.paramsOf('project/create')).toEqual([
      { name: 'app', folder: { kind: 'path', path: '/Users/ana/code/app' } },
    ]);
    expect(fixture.componentInstance.open()).toBe(false);
    expect(TestBed.inject(Router).url).toBe('/?project=p9');
    expect(TestBed.inject(ProjectsService).projects()[0]?.project.name).toBe('app');
    expect(announce).toHaveBeenLastCalledWith('Created the project app');
    expect(navigated()).toBe(1);
  });

  it('names it after the folder or repository until the person types a name', async () => {
    const { root, fixture } = await render();
    const name = (): string => find<HTMLInputElement>(root, 'new-project-name').value;
    expect(name()).toBe('');
    find(root, 'folder-picker-use').click();
    await settle(fixture);
    expect(name()).toBe('ana');
    find(root, 'new-project-change').click();
    await settle(fixture);
    find(root, 'folder-picker-folder').click();
    await settle(fixture);
    find(root, 'folder-picker-use').click();
    await settle(fixture);
    expect(name()).toBe('code');
    choose(root, 'A repository');
    await settle(fixture);
    type(root, 'new-project-url', 'https://github.com/acme/site');
    await settle(fixture);
    expect(name()).toBe('site');
    // Once typed in, it stays as the person left it.
    type(root, 'new-project-name', 'Website');
    await settle(fixture);
    type(root, 'new-project-url', 'https://github.com/acme/blog');
    await settle(fixture);
    expect(name()).toBe('Website');
    choose(root, 'A folder on this computer');
    await settle(fixture);
    expect(name()).toBe('Website');
  });

  it('takes the name typed, and changes the folder from the one picked', async () => {
    const { root, fixture, fake } = await render();
    find(root, 'folder-picker-use').click();
    await settle(fixture);
    find(root, 'new-project-change').click();
    await settle(fixture);
    // The picker opens at the folder picked, and nothing is picked until one is used again.
    expect(fake.paramsOf('folder/list').at(-1)).toEqual({ path: '/Users/ana' });
    expect(create(root).disabled).toBe(true);
    find(root, 'folder-picker-folder').click();
    await settle(fixture);
    find(root, 'folder-picker-use').click();
    await settle(fixture);
    type(root, 'new-project-name', '  Website  ');
    await settle(fixture);
    find<HTMLFormElement>(root, 'new-project-form').requestSubmit();
    await settle(fixture);
    expect(fake.paramsOf('project/create')).toEqual([
      { name: 'Website', folder: { kind: 'path', path: '/Users/ana/code' } },
    ]);
  });

  it('clones a repository, showing progress, and says in the server’s words when it cannot', async () => {
    let refuse: (error: ApiError) => void = () => undefined;
    const { root, fixture, fake, announce } = await render({
      create: () =>
        new Promise((_, reject) => {
          refuse = reject;
        }),
    });
    choose(root, 'A repository');
    await settle(fixture);
    expect(root.querySelector('app-folder-picker')).toBeNull();
    // With no git host connected, the line says where to connect one.
    expect(find(root, 'new-project-private').textContent).toContain(
      'Private repositories need a connected git host',
    );
    expect(find(root, 'new-project-connect').getAttribute('href')).toBe('/settings#accounts');
    type(root, 'new-project-url', 'https://github.com/acme/secret.git');
    await settle(fixture);
    expect(find<HTMLInputElement>(root, 'new-project-name').value).toBe('secret');
    await expectAccessible(root);
    create(root).click();
    await settle(fixture);
    expect(fake.paramsOf('project/create')).toEqual([
      { name: 'secret', folder: { kind: 'repository', url: 'https://github.com/acme/secret.git' } },
    ]);
    expect(find(root, 'new-project-cloning').textContent).toContain('Cloning the repository.');
    expect(announce).toHaveBeenCalledWith(
      'Cloning the repository. A large one can take a few minutes.',
    );
    expect(create(root).textContent.trim()).toBe('Cloning…');
    expect(create(root).getAttribute('aria-disabled')).toBe('true');
    expect(find<HTMLInputElement>(root, 'new-project-url').disabled).toBe(true);
    // Pressing it again while it clones asks nothing more.
    create(root).click();
    expect(fake.paramsOf('project/create')).toHaveLength(1);
    await expectAccessible(root);
    refuse(
      new ApiError({
        code: ErrorCode.HOST,
        message:
          'could not clone https://github.com/acme/secret.git: repository not found. If the repository is private, connect its git host first',
      }),
    );
    await settle(fixture);
    expect(find(root, 'new-project-problem').textContent).toBe(
      'Could not clone https://github.com/acme/secret.git: repository not found. If the repository is private, connect its git host first.',
    );
    expect(root.querySelector('[data-testid="new-project-cloning"]')).toBeNull();
    expect(fixture.componentInstance.open()).toBe(true);
    expect(create(root).textContent.trim()).toBe('Create');
  });

  it('keeps what the person had after a refusal, and after going to connect a git host', async () => {
    const { root, fixture, fake } = await render({
      create: () => new ApiError({ code: ErrorCode.HOST, message: 'could not clone it' }),
    });
    choose(root, 'A repository');
    await settle(fixture);
    type(root, 'new-project-url', 'https://github.com/acme/secret');
    await settle(fixture);
    create(root).click();
    await settle(fixture);
    expect(find(root, 'new-project-problem').textContent).toBe('Could not clone it.');
    fixture.componentInstance.open.set(false);
    await settle(fixture);
    fixture.componentInstance.open.set(true);
    await settle(fixture);
    expect(find<HTMLInputElement>(root, 'new-project-url').value).toBe(
      'https://github.com/acme/secret',
    );
    expect(find(root, 'new-project-problem').textContent).toBe('Could not clone it.');
    // Back from connecting a git host, the address is still there, and the hosts read again.
    find(root, 'new-project-connect').click();
    await settle(fixture);
    fixture.componentInstance.open.set(true);
    await settle(fixture);
    expect(find<HTMLInputElement>(root, 'new-project-url').value).toBe(
      'https://github.com/acme/secret',
    );
    expect(fake.paramsOf('credential/list')).toHaveLength(3);
  });

  it('says private repositories use the git host connected, when there is one', async () => {
    const { root, fixture } = await render({ accounts: [GITHUB] });
    choose(root, 'A repository');
    await settle(fixture);
    expect(find(root, 'new-project-private').textContent.trim()).toBe(
      'Private repositories use the git host you connected.',
    );
    expect(root.querySelector('[data-testid="new-project-connect"]')).toBeNull();
  });

  it('closes, and lets a drawer close, when the person goes to connect a git host', async () => {
    const { root, fixture, navigated } = await render();
    choose(root, 'A repository');
    await settle(fixture);
    find(root, 'new-project-connect').click();
    await settle(fixture);
    expect(fixture.componentInstance.open()).toBe(false);
    expect(navigated()).toBe(1);
  });

  it('makes an empty folder once the project has a name', async () => {
    const { root, fixture, fake } = await render();
    choose(root, 'Empty');
    await settle(fixture);
    expect(root.querySelector('app-folder-picker')).toBeNull();
    expect(create(root).disabled).toBe(true);
    type(root, 'new-project-name', 'Ideas');
    await settle(fixture);
    create(root).click();
    await settle(fixture);
    expect(fake.paramsOf('project/create')).toEqual([{ name: 'Ideas', folder: { kind: 'empty' } }]);
  });

  it('offers someone not on this computer a project by name alone', async () => {
    const { root, fixture, fake } = await render({ local: false });
    expect(root.querySelector('input[type="radio"]')).toBeNull();
    expect(root.querySelector('app-folder-picker')).toBeNull();
    expect(find(root, 'new-project-no-folder').textContent).toBe(
      'Its threads start in an empty folder for now; a computer for it comes later.',
    );
    expect(fake.paramsOf('folder/list')).toEqual([]);
    expect(fake.paramsOf('credential/list')).toEqual([]);
    await expectAccessible(root);
    type(root, 'new-project-name', 'Garden');
    await settle(fixture);
    create(root).click();
    await settle(fixture);
    expect(fake.paramsOf('project/create')).toEqual([{ name: 'Garden' }]);
    expect(TestBed.inject(Router).url).toBe('/?project=p9');
  });

  it('starts afresh each time it opens', async () => {
    const { root, fixture } = await render();
    choose(root, 'Empty');
    type(root, 'new-project-name', 'Ideas');
    find(root, 'new-project-cancel').click();
    await settle(fixture);
    expect(fixture.componentInstance.open()).toBe(false);
    fixture.componentInstance.open.set(true);
    await settle(fixture);
    expect(find<HTMLInputElement>(root, 'new-project-name').value).toBe('');
    expect(root.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(true);
  });
});
