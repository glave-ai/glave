import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { CHATS_LIST, PROJECTS_LIST } from '../../lib/nav';
import { NavListsService } from '../../services/nav-lists.service';
import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space, thread } from '../../testing/fixtures';
import { SpacesNavComponent } from './spaces-nav.component';

const SPACES = [space('p1', 'glave'), space('p2', 'Garden'), space('me', 'Personal', 'personal')];

/** A time on the fixtures' day, at 10:`minute`. */
function at(minute: number): string {
  return `2026-10-11T10:${String(minute).padStart(2, '0')}:00.000Z`;
}

/** The fake's threads by space: glave's newest, one waiting and one its first one started. */
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
  readonly fixture: ReturnType<typeof TestBed.createComponent<SpacesNavComponent>>;
  readonly fake: FakeSession;
}

async function render(query = '', spaces = SPACES, fake = new FakeSession()): Promise<Rendered> {
  fake
    .answer('space/list', { spaces })
    .answer('folder/list', { path: '/Users/ana', parent: '/Users', folders: [] })
    .answer('credential/list', { credentials: [], device: false })
    .answer('thread/list', (params) => ({
      threads: THREADS[(params as { space: string }).space] ?? [],
    }));
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(SpacesService).refresh();
  const fixture = TestBed.createComponent(SpacesNavComponent);
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

describe('SpacesNavComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    document.body.click();
  });

  it('lists each project with its threads, then the chats, what waits first', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const lists = Array.from(root.querySelectorAll('nav > glz-nav-list'));
    expect(lists.map((list) => list.getAttribute('data-testid'))).toEqual([
      'nav-projects',
      'nav-chats',
    ]);
    expect(texts(root, 'nav-projects-toggle')).toEqual(['Projects']);
    expect(texts(root, 'nav-chats-toggle')).toEqual(['Chats']);
    // Garden has no threads, and counts from when it was made, before glave's latest.
    expect(texts(root, 'nav-space-toggle')).toEqual(['glave', 'Garden']);
    expect(texts(root, 'nav-thread')).toEqual([
      'New chat needs you',
      'Plan the launch',
      'Check the parser',
      'Audit the dependencies',
      'Trip to Lisbon running',
    ]);
    const threads = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="nav-thread"]'));
    expect(threads[1]?.getAttribute('aria-current')).toBe('page');
    // Quiet marks: a dot for what needs the person, the word alone for running.
    expect(threads[0]?.querySelector('glz-badge')?.classList).toContain('glz-badge--dot');
    expect(threads[4]?.querySelector('glz-badge')?.classList).toContain('glz-badge--plain');
    // No folder beside a project's name, and an empty project shows no filler.
    expect(find(root, 'nav-space-toggle').querySelectorAll('glz-icon')).toHaveLength(1);
    expect(root.textContent).not.toContain('No threads');
    const add = root.querySelectorAll<HTMLAnchorElement>('[data-testid="nav-space-new"]');
    expect(add[0]?.getAttribute('aria-label')).toBe('New thread in glave');
    expect(add[0]?.getAttribute('href')).toBe('/?space=p1');
    expect(add[1]?.getAttribute('href')).toBe('/?space=p2');
    expect(find(root, 'nav-chats-new').getAttribute('href')).toBe('/');
    expect(find(root, 'nav-chats-new').getAttribute('aria-label')).toBe('New chat');
    expect(find(root, 'nav-space-more-trigger').getAttribute('aria-label')).toBe(
      'Options for glave',
    );
    await expectAccessible(root);
    threads[1]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('puts a thread another started under it, on its own line', async () => {
    const { root } = await render();
    const children = find(root, 'nav-children');
    expect(texts(children, 'nav-thread')).toEqual(['Check the parser']);
    expect(children.previousElementSibling?.textContent).toContain('Plan the launch');
  });

  it('closes and opens each heading from its row, and remembers it', async () => {
    const { root, fixture } = await render();
    const lists = TestBed.inject(NavListsService);
    const toggle = find<HTMLButtonElement>(root, 'nav-space-toggle');
    toggle.click();
    await fixture.whenStable();
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(find(root, 'nav-space-items').hidden).toBe(true);
    expect([...lists.collapsed()]).toEqual(['p1']);
    // Closed, glave still says something in it needs the person.
    const end = root.ownerDocument.getElementById(toggle.getAttribute('aria-describedby') ?? '');
    expect(end?.textContent.trim()).toBe('needs you');
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

  it('sorts a project’s threads by name from its ⋯, and remembers it', async () => {
    const rendered = await render();
    await pick(rendered, 'nav-space-more', 'choice', 'Name');
    expect(texts(rendered.root, 'nav-thread').slice(0, 4)).toEqual([
      'New chat needs you',
      'Audit the dependencies',
      'Plan the launch',
      'Check the parser',
    ]);
    expect([...TestBed.inject(NavListsService).byName()]).toEqual(['p1']);
    // By name, glave still comes first: something in it waits.
    await pick(rendered, 'nav-projects-more', 'choice', 'Name');
    expect(texts(rendered.root, 'nav-space-toggle')).toEqual(['glave', 'Garden']);
    expect([...TestBed.inject(NavListsService).byName()]).toEqual(['p1', PROJECTS_LIST]);
  });

  it('renames a project from its ⋯', async () => {
    let name = 'glave';
    const fake = new FakeSession().answer('space/rename', (params) => {
      name = (params as { name: string }).name;
      return { space: space('p1', name) };
    });
    const rendered = await render('', SPACES, fake);
    fake.answer('space/list', () => ({ spaces: [space('p1', name), ...SPACES.slice(1)] }));
    await pick(rendered, 'nav-space-more', 'action', 'Rename');
    const input = find<HTMLInputElement>(rendered.root, 'rename-input');
    expect(input.value).toBe('glave');
    input.value = 'glave app';
    input.dispatchEvent(new Event('input'));
    await rendered.fixture.whenStable();
    find<HTMLButtonElement>(rendered.root, 'rename-save').click();
    await settle(rendered);
    expect(fake.paramsOf('space/rename')).toEqual([{ space: 'p1', name: 'glave app' }]);
    expect(texts(rendered.root, 'nav-space-toggle')).toContain('glave app');
    expect(rendered.root.querySelector('[data-testid="nav-error"]')).toBeNull();
  });

  it('says why a rename failed', async () => {
    const fake = new FakeSession().answer(
      'space/rename',
      new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only its owner renames it' }),
    );
    const rendered = await render('', SPACES, fake);
    await pick(rendered, 'nav-space-more', 'action', 'Rename');
    const input = find<HTMLInputElement>(rendered.root, 'rename-input');
    input.value = 'Mine now';
    input.dispatchEvent(new Event('input'));
    await rendered.fixture.whenStable();
    find<HTMLButtonElement>(rendered.root, 'rename-save').click();
    await settle(rendered);
    expect(find(rendered.root, 'nav-error').textContent.trim()).toBe(
      'Could not rename the project: only its owner renames it',
    );
  });

  it('keeps the threads whose title matches the search, and says when none do', async () => {
    const { root, fixture } = await render('launch');
    expect(texts(root, 'nav-thread')).toEqual(['Plan the launch']);
    expect(root.querySelector('[data-testid="nav-chats"]')).toBeNull();
    expect(root.querySelectorAll('[data-testid="nav-space"]')).toHaveLength(1);
    fixture.componentRef.setInput('query', 'zebra');
    await fixture.whenStable();
    expect(texts(root, 'nav-thread')).toEqual([]);
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
    const { root, fixture } = await render('', [space('me', 'Personal', 'personal')]);
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

  it('offers nothing before the lists are read', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession()),
      ],
    });
    const fixture = TestBed.createComponent(SpacesNavComponent);
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('[data-testid="nav-start-project"]')).toBeNull();
  });
});
