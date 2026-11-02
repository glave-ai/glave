import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { type GlzServiceDetail, GlzServiceDetailComponent } from './service-detail.component';

const DETAIL: GlzServiceDetail = {
  front: 'name: release-notes\ndescription: Writes release notes.',
  text: '# Release notes\n\nRun `scripts/merged.sh`.',
  files: [
    { path: 'scripts/merged.sh', text: '#!/bin/sh\ngit log --merges', runsCode: true },
    { path: 'templates/release.md', text: '## {{version}}', runsCode: false },
  ],
  cut: null,
};

@Component({
  imports: [GlzServiceDetailComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-service-detail testId="adds" [detail]="detail()" />`,
})
class HostComponent {
  readonly detail = signal<GlzServiceDetail>(DETAIL);
}

describe('GlzServiceDetailComponent', () => {
  it('starts shut, and opened shows the text as markdown and each file as plain code', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const part = (name: string): HTMLElement | null =>
      root.querySelector(`[data-testid="adds-${name}"]`);
    const fold = root.querySelector('details');
    expect(fold?.open).toBe(false);
    expect(part('summary')?.textContent.trim()).toBe('What it adds');
    part('summary')?.click();
    await fixture.whenStable();
    expect(fold?.open).toBe(true);
    expect(part('front')?.textContent).toBe(DETAIL.front);
    expect(part('text')?.querySelector('h1')?.textContent).toBe('Release notes');
    expect(part('text')?.getAttribute('role')).toBe('region');
    const files = root.querySelectorAll('[data-testid="adds-file"]');
    expect([...files].map((file) => file.querySelector('.path')?.textContent)).toEqual([
      'scripts/merged.sh',
      'templates/release.md',
    ]);
    // Only the script is marked as one.
    expect(root.querySelectorAll('[data-testid="adds-runs"]').length).toBe(1);
    expect(part('runs')?.textContent).toBe('runs code');
    const code = root.querySelectorAll<HTMLElement>('[data-testid="adds-code"]');
    expect(code[0]?.textContent).toBe('#!/bin/sh\ngit log --merges');
    expect(code[0]?.querySelector('*:not(code)')).toBeNull();
    expect(part('cut')).toBeNull();
    await expectAccessible(root);
  });

  it("says what was cut in the server's sentence, and lists no files when there are none", async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.detail.set({
      front: '',
      text: 'Short.',
      files: [],
      cut: 'Too long to show in full here: 1 part cut short.',
    });
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    root.querySelector('details')?.setAttribute('open', '');
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="adds-cut"]')?.textContent).toBe(
      'Too long to show in full here: 1 part cut short.',
    );
    expect(root.querySelector('[data-testid="adds-front"]')).toBeNull();
    expect(root.querySelector('.files')).toBeNull();
    await expectAccessible(root);
  });
});
