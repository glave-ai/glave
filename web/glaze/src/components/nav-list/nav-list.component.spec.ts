import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Folder } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzNavItemComponent } from '../nav-item/nav-item.component';
import { GlzNavListComponent } from './nav-list.component';

@Component({
  imports: [GlzNavListComponent, GlzNavItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav aria-label="Threads">
      <glz-nav-list heading="Today" testId="today">
        <a glzNavItem href="/t/1" current>Plan the launch</a>
      </glz-nav-list>
      <glz-nav-list testId="plain"><a glzNavItem href="/t/2">Other</a></glz-nav-list>
      <glz-nav-list heading="glave" testId="space" collapsible [(expanded)]="expanded">
        <button glzNavListActions type="button" data-testid="add">New thread in glave</button>
        <a glzNavItem href="/t/3">Fix the build</a>
      </glz-nav-list>
      <glz-nav-list heading="Projects" testId="projects">
        <glz-nav-list heading="Garden" testId="project" collapsible [icon]="folder">
          <a glzNavItem href="/t/4">Plant the beans</a>
        </glz-nav-list>
      </glz-nav-list>
    </nav>
  `,
})
class HostComponent {
  readonly expanded = signal(true);
  readonly folder = Folder;
}

describe('GlzNavListComponent', () => {
  it('names its group by its heading, when it has one', async () => {
    const fixture = TestBed.createComponent(HostComponent);
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

  it('shows and hides a collapsible list from its heading, beside its actions', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="space-toggle"]');
    const items = root.querySelector<HTMLElement>('[data-testid="space-items"]');
    expect(toggle?.textContent.trim()).toBe('glave');
    expect(toggle?.getAttribute('aria-expanded')).toBe('true');
    expect(toggle?.getAttribute('aria-controls')).toBe(items?.id);
    expect(root.querySelector('[data-testid="space"]')?.getAttribute('aria-labelledby')).toBe(
      toggle?.id,
    );
    expect(root.querySelector('[data-testid="space"] .actions [data-testid="add"]')).not.toBeNull();
    await expectAccessible(root);
    toggle?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.expanded()).toBe(false);
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    expect(items?.hidden).toBe(true);
    await expectAccessible(root);
  });

  it('with an icon, makes its heading a row that leads with the icon, not a chevron', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const project = root.querySelector('[data-testid="project"]');
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="project-toggle"]');
    expect(project?.classList).toContain('glz-nav-list--row');
    expect(root.querySelector('[data-testid="space"]')?.classList).not.toContain(
      'glz-nav-list--row',
    );
    expect(toggle?.querySelectorAll('glz-icon')).toHaveLength(1);
    expect(toggle?.textContent.trim()).toBe('Garden');
    await expectAccessible(root);
    toggle?.click();
    await fixture.whenStable();
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector<HTMLElement>('[data-testid="project-items"]')?.hidden).toBe(true);
  });
});
