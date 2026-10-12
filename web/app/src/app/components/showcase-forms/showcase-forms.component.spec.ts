import { TestBed } from '@angular/core/testing';
import { GlzToastService } from '@glave/glaze';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseFormsComponent } from './showcase-forms.component';

function find<T extends HTMLElement = HTMLElement>(root: HTMLElement, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

describe('ShowcaseFormsComponent', () => {
  it('validates the reactive form on submit and creates once it is valid', async () => {
    const fixture = TestBed.createComponent(ShowcaseFormsComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    await expectAccessible(root);
    find(root, 'showcase-space-create').click();
    await fixture.whenStable();
    expect(root.textContent).toContain('Give the space a name.');
    expect(root.textContent).toContain('Read and accept the policy to go on.');

    const name = find<HTMLInputElement>(root, 'showcase-space-name');
    name.value = 'glave';
    name.dispatchEvent(new Event('input'));
    root.querySelector<HTMLInputElement>('glz-checkbox input')?.click();
    await fixture.whenStable();
    find(root, 'showcase-space-create').click();
    expect(TestBed.inject(GlzToastService).toasts()[0]?.message).toBe('Space created');
  });

  it('shows signal forms messages', async () => {
    const fixture = TestBed.createComponent(ShowcaseFormsComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const email = find<HTMLInputElement>(root, 'showcase-email');
    email.value = 'not an address';
    email.dispatchEvent(new Event('input'));
    find(root, 'showcase-email-save').click();
    await fixture.whenStable();
    expect(root.textContent).toContain('That is not an email address.');
  });
});
