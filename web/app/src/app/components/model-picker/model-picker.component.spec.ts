import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { PinnedModelsService } from '../../services/pinned-models.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS } from '../../testing/fixtures';
import { ModelPickerComponent } from './model-picker.component';

async function render(
  models = MODELS,
): Promise<ReturnType<typeof TestBed.createComponent<ModelPickerComponent>>> {
  TestBed.configureTestingModule({
    providers: provideFakeSession(new FakeSession().answer('model/list', { models })),
  });
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
    expect(trigger(root)?.classList).toContain('hope');
  });

  it('takes the person’s pick from the list, and keeps their pins', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    trigger(root)?.click();
    await fixture.whenStable();
    const items = root.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]');
    expect(items).toHaveLength(2);
    expect(
      root.querySelector('[data-testid="model-picker-search"]')?.getAttribute('placeholder'),
    ).toBe('Search models');
    await expectAccessible(root);
    items[1]?.querySelector<HTMLElement>('.pin')?.click();
    await fixture.whenStable();
    expect(TestBed.inject(PinnedModelsService).pinned()).toEqual(['hope-12b']);
    root.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]')[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('hope-12b');
  });

  it('shows nothing while the server names no models', async () => {
    const fixture = await render([]);
    expect(trigger(fixture.nativeElement as HTMLElement)).toBeNull();
  });
});
