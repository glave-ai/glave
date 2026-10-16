import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { NavService } from '../../services/nav.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { ThreadHeaderComponent } from './thread-header.component';

interface Options {
  readonly narrow?: boolean;
  readonly project?: string | null;
  readonly place?: string | null;
  readonly changes?: number;
}

async function render(options: Options = {}): Promise<{
  readonly root: HTMLElement;
  readonly renamed: () => number;
}> {
  localStorage.clear();
  const narrow = options.narrow ?? false;
  TestBed.configureTestingModule({
    providers: [
      { provide: BreakpointObserver, useValue: { observe: () => of({ matches: narrow }) } },
    ],
  });
  const fixture = TestBed.createComponent(ThreadHeaderComponent);
  fixture.componentRef.setInput('title', 'Fix the flaky test');
  fixture.componentRef.setInput('project', 'project' in options ? options.project : 'glave');
  fixture.componentRef.setInput('place', 'place' in options ? options.place : 'on your Mac');
  fixture.componentRef.setInput('changes', options.changes ?? 3);
  let renames = 0;
  fixture.componentInstance.rename.subscribe(() => (renames += 1));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, renamed: () => renames };
}

function find(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

describe('ThreadHeaderComponent', () => {
  it('shows the title, its project and where it runs, and renames from the title', async () => {
    const { root, renamed } = await render();
    expect(root.querySelector('h1[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Fix the flaky test',
    );
    expect(find(root, 'thread-meta')?.textContent.trim()).toBe('glave · on your Mac');
    // No loud status: the side nav and the card in the thread say what needs the person.
    expect(root.querySelector('glz-badge')).toBeNull();
    expect(find(root, 'thread-tools')).toBeNull();
    expect(find(root, 'page-menu')).toBeNull();
    await expectAccessible(root);
    find(root, 'thread-rename')?.click();
    expect(renamed()).toBe(1);
  });

  it('says only what it knows: a chat has no project, a thread may not know where it runs', async () => {
    const chat = await render({ project: null });
    expect(find(chat.root, 'thread-meta')?.textContent.trim()).toBe('on your Mac');
    expect(chat.root.querySelector('.folder')).toBeNull();
    TestBed.resetTestingModule();
    const unknown = await render({ place: null });
    expect(find(unknown.root, 'thread-meta')?.textContent.trim()).toBe('glave');
    TestBed.resetTestingModule();
    const neither = await render({ project: null, place: null });
    expect(find(neither.root, 'thread-meta')).toBeNull();
  });

  it('opens the tools panel on the changes from "Changes", with how many files', async () => {
    const { root } = await render();
    const changes = find(root, 'thread-changes');
    expect(changes?.textContent.replace(/\s+/gu, ' ').trim()).toBe('Changes 3');
    expect(changes?.getAttribute('aria-pressed')).toBe('false');
    changes?.click();
    const panel = TestBed.inject(ToolsPanelService);
    expect([panel.open(), panel.tab()]).toEqual([true, 'changes']);
  });

  it('shows no "Changes" when nothing changed', async () => {
    const { root } = await render({ changes: 0 });
    expect(find(root, 'thread-changes')).toBeNull();
  });

  it('on a phone, opens the side bar and the changes from buttons a finger’s size', async () => {
    const { root } = await render({ narrow: true });
    const menu = find(root, 'page-menu');
    const tools = find(root, 'thread-tools');
    expect(find(root, 'thread-changes')).toBeNull();
    expect(menu?.classList).toContain('touch');
    expect(tools?.getAttribute('aria-label')).toBe('Changes, 3 files');
    expect(tools?.getAttribute('aria-pressed')).toBe('false');
    await expectAccessible(root);
    const nav = TestBed.inject(NavService);
    const wasOpen = nav.open();
    menu?.click();
    expect(nav.open()).toBe(!wasOpen);
    tools?.click();
    const panel = TestBed.inject(ToolsPanelService);
    expect([panel.open(), panel.tab()]).toEqual([true, 'changes']);
  });

  it('on a phone with nothing changed, opens the tools', async () => {
    const { root } = await render({ narrow: true, changes: 0 });
    const tools = find(root, 'thread-tools');
    expect(tools?.getAttribute('aria-label')).toBe('Tools');
    tools?.click();
    expect(TestBed.inject(ToolsPanelService).open()).toBe(true);
  });
});
