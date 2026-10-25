import { TestBed } from '@angular/core/testing';
import type { ModelInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { EffortPickerComponent } from './effort-picker.component';

const MODELS: readonly ModelInfo[] = [
  {
    id: 'openai/gpt-6-luna',
    default: true,
    reasoning: { efforts: ['xhigh', 'high', 'medium', 'low', 'none'], default: 'medium' },
  },
  { id: 'anthropic/claude-haiku-5.5', default: false, reasoning: { efforts: ['high', 'low'] } },
  { id: 'plain', default: false },
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
  it('shows the default model’s levels, its default first chosen, and takes a pick', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('Medium');
    await expectAccessible(root);
    trigger(root)?.click();
    await fixture.whenStable();
    const options = Array.from(
      document.querySelectorAll<HTMLElement>('[data-testid="effort-picker-option"]'),
    );
    expect(options.map((option) => option.querySelector('.label')?.textContent.trim())).toEqual([
      'Extra high',
      'High',
      'Medium',
      'Low',
      'Off',
    ]);
    expect(options[2]?.querySelector('.hint')?.textContent).toContain('Default');
    options[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('high');
    expect(trigger(root)?.textContent).toContain('High');
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
    fixture.componentRef.setInput('model', 'plain');
    await fixture.whenStable();
    expect(trigger(root)).toBeNull();
  });
});
