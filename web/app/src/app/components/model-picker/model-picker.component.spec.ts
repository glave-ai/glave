import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { RecentModelsService } from '../../services/recent-models.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS, ROUTED } from '../../testing/fixtures';
import { ModelPickerComponent } from './model-picker.component';

async function render(
  models = MODELS,
): Promise<ReturnType<typeof TestBed.createComponent<ModelPickerComponent>>> {
  const fake = new FakeSession()
    .answer('model/list', { models })
    .answer('filters/get', { filters: {}, europe: [] });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(ModelPickerComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture;
}

function trigger(root: HTMLElement): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>('[data-testid="model-picker-trigger"]');
}

describe('ModelPickerComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the default until the thread or the person chooses', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('GPT-6 Luna');
    expect(trigger(root)?.textContent).not.toContain('gpt-6-luna');
    await expectAccessible(root);
    fixture.componentRef.setInput('current', 'hope-12b');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Hope');
    expect(trigger(root)?.classList).not.toContain('hope');
  });

  it('takes the person’s pick from the list by name, and lists it after the default since', async () => {
    const models = [
      ...MODELS,
      { id: 'openai/gpt-6-sol', name: 'GPT-6 Sol', maker: 'OpenAI', default: false },
    ];
    const fixture = await render(models);
    const root: HTMLElement = fixture.nativeElement;
    const names = (): readonly string[] =>
      Array.from(root.querySelectorAll('[data-testid="model-picker-option"] .label'), (label) =>
        label.textContent.trim(),
      );
    trigger(root)?.click();
    await fixture.whenStable();
    expect(names()).toEqual(['GPT-6 Luna', 'Hope', 'OpenAI · GPT-6 Sol']);
    expect(
      root.querySelector('[data-testid="model-picker-search"]')?.getAttribute('placeholder'),
    ).toBe('Search models');
    await expectAccessible(root);
    root.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[2]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('openai/gpt-6-sol');
    expect(TestBed.inject(RecentModelsService).recent()).toEqual(['openai/gpt-6-sol']);
    trigger(root)?.click();
    await fixture.whenStable();
    expect(names()).toEqual(['GPT-6 Luna', 'OpenAI · GPT-6 Sol', 'Hope']);
  });

  it('opens the model dialog, with the filters, where a router serves a model', async () => {
    const fixture = await render(ROUTED);
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('DeepSeek V4.1 Flash');
    expect(trigger(root)?.getAttribute('aria-haspopup')).toBe('dialog');
    trigger(root)?.click();
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="model-filters"]')).not.toBeNull();
    root.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('claude-haiku-5-5');
    expect(trigger(root)?.textContent).toContain('Claude Haiku 5.5');
    await expectAccessible(root);
  });

  it('shows nothing while the server names no models', async () => {
    const fixture = await render([]);
    expect(trigger(fixture.nativeElement as HTMLElement)).toBeNull();
  });
});
