import { DeferBlockBehavior, DeferBlockState, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { CHATS_LIST, PROJECTS_LIST } from '../../lib/nav';
import { NavListsService } from '../../services/nav-lists.service';
import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, thread } from '../../testing/fixtures';
import { ProjectsNavComponent } from './projects-nav.component';

const PROJECTS = [
  record('p1', 'glave'),
  record('p2', 'Garden'),
  record('me', 'Personal', 'personal'),
];

/** A time on the fixtures' day, at 10:`minute`. */
function at(minute: number): string {
  return `2026-10-11T10:${String(minute).padStart(2, '0')}:00.000Z`;
}

/** The fake's threads by project: glave's newest, one waiting and one its first one started. */
const THREADS: Readonly<Record<string, unknown>> = {
  p1: [
    thread('t1', 'Plan the launch', { updated_at: at(9) }),
    thread('t2', null, { status: 'waiting', updated_at: at(1) }),
    thread('t5', 'Check the parser', { parent: 't1', updated_at: at(8) }),
    thread('t6', 'Audit the dependencies', { updated_at: at(5) }),
  ],
  me: [thread('t3', 'Trip to Lisbon', { status: 'running' })],
};

interface Rendered {
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ProjectsNavComponent>>;
  readonly fake: FakeSession;
}

async function render(
  query = '',
  projects = PROJECTS,
  fake = new FakeSession(),
): Promise<Rendered> {
  fake
    .answer('project/list', { projects })
    .answer('folder/list', { path: '/Users/ana', parent: '/Users', folders: [] })
    .answer('credential/list', { credentials: [], device: false })
    .answer('thread/list', (params) => ({
      threads: THREADS[(params as { project: string }).project] ?? [],
    }));
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(ProjectsNavComponent);
  fixture.componentRef.setInput('current', 't1');
  fixture.componentRef.setInput('query', query);
  await fixture.whenStable();
  // The lists' menus load once the page is idle.
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture, fake };
}

function texts(root: Element, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.replace(/\s+/gu, ' ').trim(),
  );
}

function find<T extends HTMLElement>(root: ParentNode, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

/** Lets what the page started settle: the calls it made, and what they change on it. */
async function settle(rendered: Rendered): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await rendered.fixture.whenStable();
}

/** Opens the ⋯ menu `testId` (the first such) and picks its item `part` reading `text`. */
async function pick(rendered: Rendered, testId: string, part: string, text: string): Promise<void> {
  // A menu that opens before its trigger's first tick closes again (the CDK's focus check).
  await settle(rendered);
  find(rendered.root, `${testId}-trigger`).click();
  await rendered.fixture.whenStable();
  const items = Array.from(
    document.querySelectorAll<HTMLElement>(`[data-testid="${testId}-${part}"]`),
  );
  items.find((item) => item.textContent.trim() === text)?.click();
  await settle(rendered);
}

describe('ProjectsNavComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    document.body.click();
  });

  it('lists each project with its threads, then the chats, newest first', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const lists = Array.from(root.querySelectorAll('nav > glz-nav-list'));
    expect(lists.map((list) => list.getAttribute('data-testid'))).toEqual([
      'nav-projects',
      'nav-chats',
    ]);
    // Needs you comes first, while something waits.
    expect(
      find(root, 'needs-you').compareDocumentPosition(find(root, 'nav-projects')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(texts(root, 'needs-you-thread')).toEqual(['New chat glave']);
    expect(texts(root, 'nav-projects-toggle')).toEqual(['Projects']);
    expect(texts(root, 'nav-chats-toggle')).toEqual(['Chats']);
    // Garden has no threads, and counts from when it was made, before glave's latest.
    expect(texts(root, 'nav-project-toggle')).toEqual(['glave', 'Garden']);
    // What waits keeps its place in its list too.
    expect(texts(root, 'nav-thread-title')).toEqual([
      'Plan the launch',
      'Check the parser',
      'Audit the dependencies',
      'New chat',
      'Trip to Lisbon',
    ]);
    const threads = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="nav-thread"]'));
    expect(threads[0]?.getAttribute('aria-current')).toBe('page');
    // Where the time sits, a small mark for what needs the person, a ring for what runs.
    expect(threads[3]?.querySelector('[data-testid="nav-thread-waiting"]')).not.toBeNull();
    expect(threads[4]?.querySelector('[data-testid="nav-thread-running"]')).not.toBeNull();
    expect(threads[2]?.querySelector('[data-testid="nav-thread-since"]')).not.toBeNull();
    // No folder beside a project's name, and an empty project shows no filler.
    expect(find(root, 'nav-project-toggle').querySelectorAll('glz-icon')).toHaveLength(1);
    expect(root.textContent).not.toContain('No threads');
    const add = root.querySelectorAll<HTMLAnchorElement>('[data-testid="nav-project-new"]');
    expect(add[0]?.getAttribute('aria-label')).toBe('New thread in glave');
    expect(add[0]?.getAttribute('href')).toBe('/?project=p1');
    expect(add[1]?.getAttribute('href')).toBe('/?project=p2');
    expect(find(root, 'nav-chats-new').getAttribute('href')).toBe('/');
    expect(find(root, 'nav-chats-new').getAttribute('aria-label')).toBe('New chat');
    // A project's ⋯ renames it; Projects and Chats have no ⋯, since nothing sorts a list.
    expect(find(root, 'nav-project-more-trigger').getAttribute('aria-label')).toBe(
      'Options for glave',
    );
    expect(root.querySelectorAll('[aria-haspopup="menu"]')).toHaveLength(2);
    expect(root.querySelector('[data-testid^="nav-chats-more"]')).toBeNull();
    expect(root.querySelector('[data-testid^="nav-projects-more"]')).toBeNull();
    await expectAccessible(root);
    threads[0]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('puts a thread another started under it, on its own line', async () => {
    const { root } = await render();
    const children = find(root, 'nav-children');
    expect(texts(children, 'nav-thread-title')).toEqual(['Check the parser']);
    expect(children.previousElementSibling?.textContent).toContain('Plan the launch');
  });

  it('closes and opens each heading from its row, and remembers it', async () => {
    const { root, fixture } = await render();
    const lists = TestBed.inject(NavListsService);
    const toggle = find<HTMLButtonElement>(root, 'nav-project-toggle');
    toggle.click();
    await fixture.whenStable();
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(find(root, 'nav-project-items').hidden).toBe(true);
    expect([...lists.collapsed()]).toEqual(['p1']);
    // Closed, glave still says something in it needs the person.
    const end = root.ownerDocument.getElementById(toggle.getAttribute('aria-describedby') ?? '');
    expect(end?.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe('needs you');
    await expectAccessible(root);
    toggle.click();
    find<HTMLButtonElement>(root, 'nav-projects-toggle').click();
    find<HTMLButtonElement>(root, 'nav-chats-toggle').click();
    await fixture.whenStable();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(find(root, 'nav-projects-items').hidden).toBe(true);
    expect(find(root, 'nav-chats-items').hidden).toBe(true);
    expect([...lists.collapsed()]).toEqual([PROJECTS_LIST, CHATS_LIST]);
  });

  it('renames a project from its ⋯', async () => {
    let name = 'glave';
    const fake = new FakeSession().answer('project/rename', (params) => {
      name = (params as { name: string }).name;
      return { project: record('p1', name) };
    });
    const rendered = await render('', PROJECTS, fake);
    fake.answer('project/list', () => ({ projects: [record('p1', name), ...PROJECTS.slice(1)] }));
    await pick(rendered, 'nav-project-more', 'action', 'Rename');
    const input = find<HTMLInputElement>(rendered.root, 'rename-input');
    expect(input.value).toBe('glave');
    input.value = 'glave app';
    input.dispatchEvent(new Event('input'));
    await rendered.fixture.whenStable();
    find<HTMLButtonElement>(rendered.root, 'rename-save').click();
    await settle(rendered);
    expect(fake.paramsOf('project/rename')).toEqual([{ project: 'p1', name: 'glave app' }]);
    expect(texts(rendered.root, 'nav-project-toggle')).toContain('glave app');
    expect(rendered.root.querySelector('[data-testid="nav-error"]')).toBeNull();
  });

  it('says why a rename failed', async () => {
    const fake = new FakeSession().answer(
      'project/rename',
      new ApiError({
        code: ErrorCode.INVALID_PARAMS,
        message: 'there is a project called "glave" already; choose another name',
      }),
    );
    const rendered = await render('', PROJECTS, fake);
    await pick(rendered, 'nav-project-more', 'action', 'Rename');
    const input = find<HTMLInputElement>(rendered.root, 'rename-input');
    input.value = 'Mine now';
    input.dispatchEvent(new Event('input'));
    await rendered.fixture.whenStable();
    find<HTMLButtonElement>(rendered.root, 'rename-save').click();
    await settle(rendered);
    expect(find(rendered.root, 'nav-error').textContent.trim()).toBe(
      'Could not rename the project: there is a project called "glave" already; choose another name',
    );
  });

  it('keeps the threads whose title matches the search, and says when none do', async () => {
    const { root, fixture } = await render('launch');
    expect(texts(root, 'nav-thread-title')).toEqual(['Plan the launch']);
    // What waits is not found, so Needs you is gone.
    expect(root.querySelector('[data-testid="needs-you"]')).toBeNull();
    expect(root.querySelector('[data-testid="nav-chats"]')).toBeNull();
    expect(root.querySelectorAll('[data-testid="nav-project"]')).toHaveLength(1);
    fixture.componentRef.setInput('query', 'zebra');
    await fixture.whenStable();
    expect(texts(root, 'nav-thread-title')).toEqual([]);
    expect(root.querySelector('[data-testid="nav-no-match"]')?.textContent).toBe(
      'No threads match',
    );
    await expectAccessible(root);
  });

  it('opens New project from the + beside Projects', async () => {
    const { root, fixture } = await render();
    const add = find<HTMLButtonElement>(root, 'nav-new-project');
    expect(add.closest('[data-testid="nav-projects"]')).not.toBeNull();
    expect(add.getAttribute('aria-label')).toBe('New project');
    // It is loaded the first time it opens.
    expect(root.querySelector('[data-testid="new-project"]')).toBeNull();
    add.click();
    await fixture.whenStable();
    expect(root.querySelector<HTMLDialogElement>('[data-testid="new-project"]')?.open).toBe(true);
    expect(root.querySelector('[data-testid="nav-start-project"]')).toBeNull();
  });

  it('with no projects yet, offers one quiet row to start one instead of a heading', async () => {
    const { root, fixture } = await render('', [record('me', 'Personal', 'personal')]);
    expect(root.querySelector('[data-testid="nav-projects"]')).toBeNull();
    expect(texts(root, 'nav-chats-toggle')).toEqual(['Chats']);
    const offer = find<HTMLButtonElement>(root, 'nav-start-project');
    expect(offer.textContent.trim()).toBe('Start a project');
    await expectAccessible(root);
    offer.click();
    await fixture.whenStable();
    expect(root.querySelector<HTMLDialogElement>('[data-testid="new-project"]')?.open).toBe(true);
    // A search is about threads: the offer steps aside.
    fixture.componentRef.setInput('query', 'trip');
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="nav-start-project"]')).toBeNull();
  });

  it('says on a project whose folder is gone what the server says, and what can be done', async () => {
    const gone = "This project's folder isn't there anymore.";
    const rendered = await render();
    rendered.fake.answer('project/list', {
      projects: PROJECTS,
      gone: [{ project: 'p2', message: gone }],
    });
    await TestBed.inject(ProjectsService).refresh();
    await settle(rendered);
    const { root } = rendered;
    expect(texts(root, 'nav-gone-message')).toEqual([gone]);
    expect(texts(root, 'nav-gone-find')).toEqual(['Find it']);
    expect(texts(root, 'nav-gone-remove')).toEqual(['Remove the project']);
    const garden = Array.from(root.querySelectorAll('[data-testid="nav-project"]')).find((each) =>
      each.textContent.includes('Garden'),
    );
    expect(garden?.querySelector('[data-testid="nav-gone"]')).not.toBeNull();
    const mark = garden?.querySelector('[data-testid="nav-project-end"] [role="img"]');
    expect(mark?.getAttribute('aria-label')).toBe(gone);
    expect(mark?.classList).toContain('warning');
    await expectAccessible(root);
  });

  it('says so when a dialog opened cannot load its code', async () => {
    TestBed.configureTestingModule({ deferBlockBehavior: DeferBlockBehavior.Manual });
    const { root, fixture } = await render();
    // New project opened, as its code failed to load: it and Rename are the last two blocks,
    // after each project's menu.
    const newProject = (await fixture.getDeferBlocks()).at(-2);
    await newProject?.render(DeferBlockState.Error);
    expect(
      Array.from(root.querySelectorAll('[role="alert"]'), (each) => each.textContent.trim()),
    ).toEqual(['This part of glave could not load. Reload the page to try again.']);
    await expectAccessible(root);
  });

  it('offers nothing before the lists are read', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession()),
      ],
    });
    const fixture = TestBed.createComponent(ProjectsNavComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="nav-start-project"]')).toBeNull();
  });
});
