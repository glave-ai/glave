import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { type ShownThought, ThoughtService } from '../../services/thought.service';
import { PanelThoughtComponent } from './panel-thought.component';

async function render(shown: ShownThought | null): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [{ provide: ThoughtService, useValue: { shown: signal(shown) } }],
  });
  const fixture = TestBed.createComponent(PanelThoughtComponent);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('PanelThoughtComponent', () => {
  it('shows the reasoning chosen as written, its lines kept, with how long it took', async () => {
    const root = await render({
      key: 'message-2',
      text: 'The port is 5173.\nStopping it.',
      thought: { headline: 'Stopping it.', ms: 4250 },
    });
    expect(root.querySelector('h3')?.textContent).toContain('Reasoning');
    expect(root.querySelector('[data-testid="panel-thought-took"]')?.textContent).toBe('4s');
    const text = root.querySelector<HTMLElement>('[data-testid="panel-thought-text"]');
    expect(text?.textContent).toBe('The port is 5173.\nStopping it.');
    expect(text?.getAttribute('dir')).toBe('auto');
    await expectAccessible(root);
  });

  it('says no time the server never gave', async () => {
    const root = await render({
      key: 'message-2',
      text: 'Old.',
      thought: { headline: null, ms: null },
    });
    expect(root.querySelector('[data-testid="panel-thought-text"]')?.textContent).toBe('Old.');
    expect(root.querySelector('[data-testid="panel-thought-took"]')).toBeNull();
  });

  it('shows nothing while none is chosen', async () => {
    const root = await render(null);
    expect(root.querySelector('[data-testid="panel-thought"]')).toBeNull();
  });
});
