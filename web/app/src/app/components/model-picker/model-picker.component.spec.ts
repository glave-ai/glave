import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

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
  afterEach(() => {
    document.body.click();
  });

  it('shows the default until the thread or the person chooses', async () => {
    const fixture = await render();
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('gpt-6-luna');
    await expectAccessible(root);
    fixture.componentRef.setInput('current', 'hope-12b');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Hope');
    expect(trigger(root)?.classList).toContain('hope');
  });

  it('takes the person’s pick from the menu', async () => {
    const fixture = await render();
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    const items = document.querySelectorAll<HTMLElement>('[data-testid="model-picker-option"]');
    expect(items).toHaveLength(2);
    items[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('hope-12b');
  });

  it('shows nothing while the server names no models', async () => {
    const fixture = await render([]);
    expect(trigger(fixture.nativeElement as HTMLElement)).toBeNull();
  });
});
