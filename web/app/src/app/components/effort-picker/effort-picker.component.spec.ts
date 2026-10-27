import { TestBed } from '@angular/core/testing';
import type { ModelInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { UNASKED } from '../../lib/effort';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { EffortPickerComponent } from './effort-picker.component';

const MODELS: readonly ModelInfo[] = [
  {
    id: 'openai/gpt-6-luna',
    name: 'GPT-6 Luna',
    default: true,
    reasoning: { efforts: ['xhigh', 'high', 'medium', 'low', 'none'], default: 'medium' },
  },
  {
    id: 'anthropic/claude-haiku-5.5',
    name: 'Claude Haiku 5.5',
    default: false,
    reasoning: { efforts: ['high', 'low'] },
  },
  { id: 'plain', name: 'Plain', default: false },
];

async function render(): Promise<
  ReturnType<typeof TestBed.createComponent<EffortPickerComponent>>
> {
  TestBed.configureTestingModule({
    providers: provideFakeSession(new FakeSession().answer('model/list', { models: MODELS })),
  });
  const fixture = TestBed.createComponent(EffortPickerComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture;
}

function trigger(root: HTMLElement): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>('[data-testid="effort-picker-trigger"]');
}

describe('EffortPickerComponent', () => {
  it('offers Default, then the default model’s levels, Default first chosen, and takes a pick', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    // Default, at the level the server says it is.
    expect(trigger(root)?.textContent).toContain('Medium');
    await expectAccessible(root);
    const options = async (): Promise<readonly HTMLElement[]> => {
      trigger(root)?.click();
      await fixture.whenStable();
      return Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="effort-picker-option"]'),
      );
    };
    const offered = await options();
    expect(offered.map((option) => option.querySelector('.label')?.textContent.trim())).toEqual([
      'Default',
      'Extra high',
      'High',
      'Medium',
      'Low',
      'Off',
    ]);
    expect(offered[0]?.querySelector('.hint')?.textContent).toContain('Medium');
    expect(offered[0]?.getAttribute('aria-checked')).toBe('true');
    offered[2]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('high');
    expect(trigger(root)?.textContent).toContain('High');
    // Back to Default, which the next turn sends.
    (await options())[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe(UNASKED);
    expect(trigger(root)?.textContent).toContain('Medium');
  });

  it('follows the model: its own levels, the thread’s level only where offered, none to choose', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    fixture.componentRef.setInput('model', 'anthropic/claude-haiku-5.5');
    fixture.componentRef.setInput('current', 'low');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Low');
    fixture.componentRef.setInput('current', 'none');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).not.toContain('Off');
    // No level is sent, so the model reasons as its maker set it.
    expect(trigger(root)?.textContent).toContain('Default');
    // A model that does not offer the thread's level is sent its default, which shows.
    fixture.componentRef.setInput('model', 'openai/gpt-6-luna');
    fixture.componentRef.setInput('current', 'max');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Medium');
    fixture.componentRef.setInput('model', 'plain');
    await fixture.whenStable();
    expect(trigger(root)).toBeNull();
  });
});
