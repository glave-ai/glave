import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzPageHeaderComponent } from './page-header.component';

@Component({
  imports: [GlzPageHeaderComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-page-header testId="page" [backLabel]="backLabel()" (back)="wentBack()">
      <button glzPageHeaderStart type="button">Menu</button>
    </glz-page-header>
  `,
})
class HostComponent {
  readonly backLabel = signal<string | null>('Back');
  readonly went = signal(0);

  wentBack(): void {
    this.went.update((went) => went + 1);
  }
}

async function render(): Promise<{
  readonly root: HTMLElement;
  readonly host: HostComponent;
  readonly settle: () => Promise<void>;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  return {
    root: fixture.nativeElement as HTMLElement,
    host: fixture.componentInstance,
    settle: () => fixture.whenStable(),
  };
}

describe('GlzPageHeaderComponent', () => {
  it('is a header with what the app put at its start, then Back with its arrow', async () => {
    const { root } = await render();
    const start = root.querySelector('header glz-top-bar .start');
    const buttons = [...(start?.querySelectorAll('button') ?? [])];
    expect(buttons.map((button) => button.textContent.trim())).toEqual(['Menu', 'Back']);
    const back = root.querySelector('[data-testid="page-back"]');
    expect(back).toBe(buttons[1]);
    expect(back?.getAttribute('type')).toBe('button');
    // The arrow points along the line, so it turns round in right-to-left text.
    expect(back?.querySelector('glz-icon')?.classList).toContain('glz-mirrored');
    await expectAccessible(root);
  });

  it('is reached from the keyboard, and emits back when Back is pressed', async () => {
    const { root, host } = await render();
    const back = root.querySelector<HTMLButtonElement>('[data-testid="page-back"]');
    back?.focus();
    expect(document.activeElement).toBe(back);
    back?.click();
    expect(host.went()).toBe(1);
  });

  it('has no Back control without its words', async () => {
    const { root, host, settle } = await render();
    host.backLabel.set(null);
    await settle();
    expect(root.querySelector('[data-testid="page-back"]')).toBeNull();
    expect(root.querySelector('.start')?.textContent.trim()).toBe('Menu');
    await expectAccessible(root);
  });
});
