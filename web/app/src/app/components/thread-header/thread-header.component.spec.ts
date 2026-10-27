import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { NavService } from '../../services/nav.service';
import { PanelService } from '../../services/panel.service';
import { ThreadHeaderComponent } from './thread-header.component';

interface Options {
  readonly narrow?: boolean;
  readonly project?: string | null;
  readonly panel?: boolean;
}

async function render(options: Options = {}): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadHeaderComponent>>;
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
  fixture.componentRef.setInput('panel', options.panel ?? true);
  let renames = 0;
  fixture.componentInstance.rename.subscribe(() => (renames += 1));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture, renamed: () => renames };
}

function find(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

describe('ThreadHeaderComponent', () => {
  it('shows the title and its project, and renames from the title', async () => {
    const { root, renamed } = await render();
    expect(root.querySelector('h1[data-testid="thread-title"]')?.textContent.trim()).toBe(
      'Fix the flaky test',
    );
    expect(find(root, 'thread-meta')?.textContent.trim()).toBe('glave');
    // No loud status: the side nav and the card in the thread say what needs the person.
    expect(root.querySelector('glz-badge')).toBeNull();
    expect(find(root, 'page-menu')).toBeNull();
    await expectAccessible(root);
    find(root, 'thread-rename')?.click();
    expect(renamed()).toBe(1);
  });

  it('says no project for a chat', async () => {
    const { root } = await render({ project: null });
    expect(find(root, 'thread-meta')).toBeNull();
  });

  it('shows and hides the thread’s panel from a pressed toggle, and has none without one', async () => {
    const { root } = await render();
    const button = find(root, 'thread-panel');
    const panel = TestBed.inject(PanelService);
    expect(button?.getAttribute('aria-label')).toBe('Show or hide the thread panel');
    expect(button?.getAttribute('aria-pressed')).toBe('true');
    expect(button?.hasAttribute('aria-expanded')).toBe(false);
    expect(button?.getAttribute('aria-keyshortcuts')).toMatch(/^(Meta|Control)\+\.$/u);
    expect(button?.classList).not.toContain('touch');
    button?.click();
    expect(panel.showing()).toBe(false);
    TestBed.resetTestingModule();
    const chat = await render({ panel: false });
    expect(find(chat.root, 'thread-panel')).toBeNull();
  });

  it('on a phone, opens the side bar and the panel’s sheet from buttons a finger’s size', async () => {
    const { root, fixture } = await render({ narrow: true });
    const menu = find(root, 'page-menu');
    const button = find(root, 'thread-panel');
    expect(menu?.classList).toContain('touch');
    expect(button?.classList).toContain('touch');
    // It opens a sheet over the page: expanded or not, never pressed.
    expect(button?.getAttribute('aria-expanded')).toBe('false');
    expect(button?.hasAttribute('aria-pressed')).toBe(false);
    await expectAccessible(root);
    const nav = TestBed.inject(NavService);
    const wasOpen = nav.open();
    menu?.click();
    expect(nav.open()).toBe(!wasOpen);
    button?.click();
    await fixture.whenStable();
    expect(TestBed.inject(PanelService).showing()).toBe(true);
    expect(button?.getAttribute('aria-expanded')).toBe('true');
  });
});
