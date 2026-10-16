import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import type { GlzMenuOption } from '../menu-select/menu-select.component';
import { GlzPickerComponent } from './picker.component';

const MODELS: readonly GlzMenuOption[] = [
  { value: 'anthropic/claude-haiku-5.5', label: 'Anthropic · Claude Haiku 5.5', hint: 'Default' },
  { value: 'openai/gpt-6-luna', label: 'OpenAI · GPT-6 Luna' },
  { value: 'deepseek/deepseek-v4.1-flash', label: 'DeepSeek · DeepSeek V4.1 Flash' },
  { value: 'hope-12b', label: 'Hope', hope: true },
];

@Component({
  imports: [GlzPickerComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-picker
      testId="model"
      label="Model"
      searchLabel="Search models"
      [options]="options()"
      [(value)]="value"
    />
  `,
})
class HostComponent {
  readonly options = signal(MODELS);
  readonly value = signal<string | null>('anthropic/claude-haiku-5.5');
}

function part(root: HTMLElement, name: string): HTMLElement {
  const found = root.querySelector<HTMLElement>(`[data-testid="model-${name}"]`);
  if (found === null) {
    throw new Error(`no ${name}`);
  }
  return found;
}

function labels(root: HTMLElement): readonly string[] {
  return Array.from(root.querySelectorAll('[data-testid="model-option"] .label'), (label) =>
    label.textContent.trim(),
  );
}

/** The CDK's key manager reads `keyCode`. */
const KEY_CODES: Readonly<Record<string, number>> = {
  ArrowDown: 40,
  ArrowUp: 38,
  Enter: 13,
  Escape: 27,
};

function press(element: HTMLElement, key: string): void {
  element.dispatchEvent(
    new KeyboardEvent('keydown', { key, keyCode: KEY_CODES[key], bubbles: true, cancelable: true }),
  );
}

async function opened(): Promise<{
  readonly root: HTMLElement;
  readonly host: HostComponent;
  readonly stable: () => Promise<unknown>;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  const root: HTMLElement = fixture.nativeElement;
  part(root, 'trigger').click();
  await fixture.whenStable();
  return { root, host: fixture.componentInstance, stable: () => fixture.whenStable() };
}

describe('GlzPickerComponent', () => {
  it('shows the choice on its button, and opens a searchable list on it', async () => {
    const { root } = await opened();
    const trigger = part(root, 'trigger');
    const search = part(root, 'search');
    expect(trigger.textContent).toContain('Anthropic · Claude Haiku 5.5');
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    // Focus moves to the search box in a browser (end to end); jsdom keeps a popover unrendered.
    expect(search.getAttribute('role')).toBe('combobox');
    expect(labels(root)).toHaveLength(4);
    const active = search.getAttribute('aria-activedescendant');
    expect(root.querySelector(`[id="${active ?? ''}"]`)?.getAttribute('aria-selected')).toBe(
      'true',
    );
    await expectAccessible(root);
  });

  it('filters as the person types, and picks with the keys', async () => {
    const { root, host, stable } = await opened();
    const search = part(root, 'search') as HTMLInputElement;
    // Typed as it is: no autocorrect, capitals or spelling turn "gpt" into another word.
    expect(search.getAttribute('autocorrect')).toBe('off');
    expect(search.getAttribute('autocapitalize')).toBe('off');
    expect(search.getAttribute('spellcheck')).toBe('false');
    search.value = 'gpt';
    search.dispatchEvent(new Event('input'));
    await stable();
    expect(labels(root)).toEqual(['OpenAI · GPT-6 Luna']);
    const match = root.querySelector<HTMLElement>('[data-testid="model-option"]');
    expect(match?.getAttribute('role')).toBe('option');
    expect(search.getAttribute('aria-activedescendant')).toBe(match?.id);
    await expectAccessible(root);
    search.value = 'zebra';
    search.dispatchEvent(new Event('input'));
    await stable();
    expect(labels(root)).toEqual([]);
    expect(part(root, 'empty').textContent.trim()).toBe('No matches');
    search.value = '';
    search.dispatchEvent(new Event('input'));
    await stable();
    press(search, 'ArrowDown');
    press(search, 'ArrowDown');
    press(search, 'Enter');
    await stable();
    expect(host.value()).toBe('deepseek/deepseek-v4.1-flash');
    expect(document.activeElement).toBe(part(root, 'trigger'));
    expect(part(root, 'trigger').getAttribute('aria-expanded')).toBe('false');
  });

  it('picks with a click', async () => {
    const { root, host } = await opened();
    root.querySelectorAll<HTMLElement>('[data-testid="model-option"]')[3]?.click();
    expect(host.value()).toBe('hope-12b');
  });

  it('closes on Escape without picking', async () => {
    const { root, host, stable } = await opened();
    press(part(root, 'search'), 'ArrowDown');
    press(part(root, 'search'), 'Escape');
    await stable();
    expect(host.value()).toBe('anthropic/claude-haiku-5.5');
    expect(document.activeElement).toBe(part(root, 'trigger'));
  });

  it('opens on the side with the most room when there is too little above', async () => {
    const supports = vi.spyOn(CSS, 'supports').mockReturnValue(true);
    try {
      const { root } = await opened();
      const popover = root.querySelector<HTMLElement>('[popover]');
      expect(popover?.style.getPropertyValue('position-area')).toBe('block-start');
      expect(popover?.style.getPropertyValue('position-try-order')).toBe('most-height');
    } finally {
      supports.mockRestore();
    }
  });
});
