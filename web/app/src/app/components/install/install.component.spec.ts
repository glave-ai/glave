import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import type { InstallOffer } from '../../lib/install';
import { InstallService } from '../../services/install.service';
import { InstallComponent } from './install.component';

async function render(offer: InstallOffer): Promise<{
  readonly root: HTMLElement;
  readonly install: ReturnType<typeof vi.fn>;
}> {
  const install = vi.fn(() => Promise.resolve());
  TestBed.configureTestingModule({
    providers: [{ provide: InstallService, useValue: { offer: signal(offer), install } }],
  });
  const fixture = TestBed.createComponent(InstallComponent);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, install };
}

describe('InstallComponent', () => {
  it('shows nothing where there is nothing to offer', async () => {
    const { root } = await render('none');
    expect(root.querySelector('[data-testid="install-app"]')).toBeNull();
  });

  it('shows the browser’s own prompt on a click', async () => {
    const { root, install } = await render('prompt');
    const button = root.querySelector<HTMLButtonElement>('[data-testid="install-app"]');
    expect(button?.textContent.trim()).toBe('Install app');
    expect(root.querySelector('[data-testid="install-how"]')).toBeNull();
    await expectAccessible(root);
    button?.click();
    expect(install).toHaveBeenCalledTimes(1);
  });

  it('explains Safari’s one step in a popover', async () => {
    const { root, install } = await render('safari-mac');
    const button = root.querySelector<HTMLButtonElement>('[data-testid="install-app"]');
    const how = root.querySelector<HTMLElement>('[data-testid="install-how"]');
    expect(button?.getAttribute('popovertarget')).toBe(how?.id);
    expect(how?.hasAttribute('popover')).toBe(true);
    expect(how?.textContent).toContain('File, then Add to Dock');
    // Opened, it is placed above the button.
    how?.dispatchEvent(Object.assign(new Event('toggle'), { newState: 'open' }));
    expect(how?.style.position).toBe('fixed');
    await expectAccessible(root);
    expect(install).not.toHaveBeenCalled();
  });

  it('says where Safari keeps it on a phone', async () => {
    const { root } = await render('safari-ios');
    expect(root.querySelector('[data-testid="install-how"]')?.textContent).toContain(
      'Add to Home Screen',
    );
  });
});
