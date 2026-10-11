import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzTabComponent } from '../tab/tab.component';
import { GlzTabsComponent } from './tabs.component';

@Component({
  imports: [GlzTabsComponent, GlzTabComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-tabs testId="tabs" label="Details" [(selected)]="tab">
      <glz-tab label="Conversation">Talk</glz-tab>
      <glz-tab label="Events">Every event</glz-tab>
      <glz-tab label="Cost">Money</glz-tab>
    </glz-tabs>
  `,
})
class HostComponent {
  readonly tab = signal(0);
}

function tabs(root: HTMLElement): readonly HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('[data-testid="tabs-tab"]'));
}

function panel(root: HTMLElement): HTMLElement | null {
  return root.querySelector('[data-testid="tabs-panel"]');
}

describe('GlzTabsComponent', () => {
  it('shows the selected tab, labelled by it, with one tab in the tab order', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [first, second] = tabs(root);
    expect(first?.getAttribute('aria-selected')).toBe('true');
    expect(first?.tabIndex).toBe(0);
    expect(second?.tabIndex).toBe(-1);
    expect(panel(root)?.textContent).toContain('Talk');
    expect(panel(root)?.getAttribute('aria-labelledby')).toBe(first?.id);
    expect(first?.getAttribute('aria-controls')).toBe(panel(root)?.id);
    await expectAccessible(root);

    second?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.tab()).toBe(1);
    expect(panel(root)?.textContent).toContain('Every event');
  });

  it('moves with arrow keys, Home and End, focusing the tab', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const codes: Readonly<Record<string, number>> = {
      ArrowLeft: 37,
      ArrowRight: 39,
      Home: 36,
      End: 35,
      a: 65,
    };
    const press = async (key: string): Promise<void> => {
      root
        .querySelector('[role="tab"][aria-selected="true"]')
        ?.dispatchEvent(new KeyboardEvent('keydown', { key, keyCode: codes[key], bubbles: true }));
      await fixture.whenStable();
    };
    await press('ArrowLeft');
    expect(fixture.componentInstance.tab()).toBe(2);
    expect(document.activeElement).toBe(tabs(root)[2]);
    await press('ArrowRight');
    expect(fixture.componentInstance.tab()).toBe(0);
    await press('End');
    expect(fixture.componentInstance.tab()).toBe(2);
    await press('Home');
    expect(fixture.componentInstance.tab()).toBe(0);
    await press('a');
    expect(fixture.componentInstance.tab()).toBe(0);
  });
});
