import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzBreadcrumbComponent } from '../breadcrumb/breadcrumb.component';
import { GlzBreadcrumbsComponent } from './breadcrumbs.component';

@Component({
  imports: [GlzBreadcrumbsComponent, GlzBreadcrumbComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-breadcrumbs>
      <glz-breadcrumb><a href="/spaces/glave">glave</a></glz-breadcrumb>
      <glz-breadcrumb current>Plan the launch</glz-breadcrumb>
    </glz-breadcrumbs>
  `,
})
class HostComponent {}

describe('GlzBreadcrumbsComponent', () => {
  it('is a named landmark listing the trail', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('nav')?.getAttribute('aria-label')).toBe('Breadcrumb');
    expect(root.querySelectorAll('[role="listitem"]')).toHaveLength(2);
    await expectAccessible(root);
  });
});
