import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { Hand } from 'lucide';

import { GlzIconComponent } from '../icon/icon.component';
import { GlzNavItemComponent } from '../nav-item/nav-item.component';
import { GlzNavListComponent } from './nav-list.component';

@Component({
  imports: [GlzIconComponent, GlzNavListComponent, GlzNavItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav aria-label="Threads">
      <glz-nav-list heading="Today" testId="today">
        <a glzNavItem href="/t/1" current>Plan the launch</a>
      </glz-nav-list>
      <glz-nav-list testId="plain"><a glzNavItem href="/t/2">Other</a></glz-nav-list>
      <glz-nav-list heading="Atlas" testId="space" collapsible [(expanded)]="expanded">
        <button glzNavListActions type="button" data-testid="add">New item in Atlas</button>
        <glz-icon glzNavListEnd label="needs you" [icon]="mark" />
        <a glzNavItem href="/t/3">Fix the build</a>
      </glz-nav-list>
      <glz-nav-list heading="Projects" testId="projects" collapsible>
        <button glzNavListActions type="button" data-testid="new-project">New project</button>
        <glz-nav-list heading="Garden" testId="project" collapsible>
          <a glzNavItem href="/t/4">Plant the beans</a>
        </glz-nav-list>
      </glz-nav-list>
    </nav>
  `,
})
class HostComponent {
  readonly expanded = signal(true);
  readonly mark = Hand;
}

function render(): ReturnType<typeof TestBed.createComponent<HostComponent>> {
  return TestBed.createComponent(HostComponent);
}

describe('GlzNavListComponent', () => {
  it('names its group by its heading, when it has one', async () => {
    const fixture = render();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const today = root.querySelector('[data-testid="today"]');
    const heading = today?.querySelector('.heading');
    expect(heading?.textContent).toBe('Today');
    expect(today?.getAttribute('aria-labelledby')).toBe(heading?.id);
    expect(root.querySelector('[data-testid="plain"]')?.hasAttribute('aria-labelledby')).toBe(
      false,
    );
    await expectAccessible(root);
  });

  it('hides the caret and actions until keyboard focus is on the heading', async () => {
    const fixture = render();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const add = root.querySelector<HTMLButtonElement>('[data-testid="add"]');
    const caret = root.querySelector('[data-testid="space-caret"]') ?? root;
    const actions = root.querySelector('[data-testid="space"] .actions') ?? root;
    const shown = (): readonly string[] =>
      [caret, actions].map((element) => getComputedStyle(element).opacity);
    expect(shown()).toEqual(['0', '0']);
    // On an action the app put there, too. jsdom counts any focus as the keyboard's; that a
    // click's focus shows nothing is checked in browsers, end to end.
    add?.focus();
    expect(shown()).toEqual(['1', '1']);
    add?.blur();
    expect(shown()).toEqual(['0', '0']);
  });

  it('shows and hides a collapsible list from its heading, the caret after the name', async () => {
    const fixture = render();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="space-toggle"]');
    const items = root.querySelector<HTMLElement>('[data-testid="space-items"]');
    expect(toggle?.textContent.trim()).toBe('Atlas');
    expect(toggle?.lastElementChild?.getAttribute('data-testid')).toBe('space-caret');
    expect(toggle?.getAttribute('aria-expanded')).toBe('true');
    expect(toggle?.getAttribute('aria-controls')).toBe(items?.id);
    expect(root.querySelector('[data-testid="space"]')?.getAttribute('aria-labelledby')).toBe(
      toggle?.id,
    );
    // The actions sit beside the toggle, never inside it.
    expect(toggle?.querySelector('button')).toBeNull();
    expect(root.querySelector('[data-testid="space"] .actions [data-testid="add"]')).not.toBeNull();
    await expectAccessible(root);
    toggle?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.expanded()).toBe(false);
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    expect(items?.hidden).toBe(true);
    await expectAccessible(root);
  });

  it('puts a mark at the heading’s end, which describes its toggle', async () => {
    const fixture = render();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const toggle = root.querySelector('[data-testid="space-toggle"]');
    const end = root.ownerDocument.getElementById(toggle?.getAttribute('aria-describedby') ?? '');
    expect(end?.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe('needs you');
    expect(end?.previousElementSibling?.classList).toContain('actions');
  });

  it('makes a list inside another a group, and spaces the groups apart', async () => {
    const fixture = render();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const projects = root.querySelector('[data-testid="projects"]');
    const project = root.querySelector('[data-testid="project"]');
    expect(projects?.classList).toContain('glz-nav-list--groups');
    expect(projects?.classList).not.toContain('glz-nav-list--group');
    expect(project?.classList).toContain('glz-nav-list--group');
    expect(project?.classList).not.toContain('glz-nav-list--groups');
    expect(root.querySelector('[data-testid="space"]')?.classList).not.toContain(
      'glz-nav-list--group',
    );
    // The outer list's own actions stay in its own header, and its end, with no mark, is empty.
    const header = projects?.querySelector(':scope > .header');
    const end = header?.querySelector('.end');
    expect(Array.from(end?.childNodes ?? []).map((node) => node.nodeType)).not.toContain(
      Node.TEXT_NODE,
    );
    expect(end?.children).toHaveLength(0);
    expect(header?.querySelector('.heading')?.textContent.trim()).toBe('Projects');
    expect(header?.querySelectorAll('.actions button')).toHaveLength(1);
    await expectAccessible(root);
  });
});
