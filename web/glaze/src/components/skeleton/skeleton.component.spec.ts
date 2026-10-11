import { TestBed } from '@angular/core/testing';

import { GlzSkeletonComponent } from './skeleton.component';

describe('GlzSkeletonComponent', () => {
  it('takes its shape and size and stays hidden from screen readers', async () => {
    const fixture = TestBed.createComponent(GlzSkeletonComponent);
    fixture.componentRef.setInput('shape', 'circle');
    fixture.componentRef.setInput('width', '3rem');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('aria-hidden')).toBe('true');
    expect(host.classList).toContain('glz-skeleton-circle');
    expect(host.style.getPropertyValue('--glz-skeleton-width')).toBe('3rem');
  });
});
