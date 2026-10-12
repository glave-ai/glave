import { Clipboard } from '@angular/cdk/clipboard';
import { TestBed } from '@angular/core/testing';
import { GlzToastService } from '@glave/glaze';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseActionsComponent } from './showcase-actions.component';

describe('ShowcaseActionsComponent', () => {
  it('shows the actions and display components, and confirms a copy', async () => {
    TestBed.configureTestingModule({
      providers: [{ provide: Clipboard, useValue: { copy: () => true } }],
    });
    const fixture = TestBed.createComponent(ShowcaseActionsComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('section#actions')).not.toBeNull();
    expect(root.querySelectorAll('glz-badge')).toHaveLength(5);
    root.querySelector<HTMLButtonElement>('[data-testid="showcase-copy"]')?.click();
    expect(TestBed.inject(GlzToastService).toasts()[0]?.message).toBe('Copied');
    await expectAccessible(root);
  });
});
