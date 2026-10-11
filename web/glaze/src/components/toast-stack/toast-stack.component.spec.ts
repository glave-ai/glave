import { TestBed } from '@angular/core/testing';

import { GlzToastService } from '../../services/toast.service';
import { expectAccessible } from '../../testing/axe';
import { popoverOpen } from '../../testing/platform';
import { GlzToastStackComponent } from './toast-stack.component';

describe('GlzToastStackComponent', () => {
  it('shows the toasts with their tone, action and dismiss button', async () => {
    const toasts = TestBed.inject(GlzToastService);
    const fixture = TestBed.createComponent(GlzToastStackComponent);
    const run = vi.fn();
    toasts.show('Archived', { tone: 'success', action: { label: 'Undo', run } });
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(popoverOpen(root)).toBe(true);
    const toast = root.querySelector('[data-testid="toast"]');
    expect(toast?.classList).toContain('tone-success');
    expect(toast?.textContent).toContain('Archived');
    expect(root.getAttribute('role')).toBe('region');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="toast-action"]')?.click();
    await fixture.whenStable();
    expect(run).toHaveBeenCalledOnce();
    expect(root.querySelector('[data-testid="toast"]')).toBeNull();
    expect(popoverOpen(root)).toBe(false);
  });
});
