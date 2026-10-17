import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import {
  type GlzTrustAnswer,
  type GlzTrustItem,
  GlzTrustCardComponent,
} from './trust-card.component';

const ITEMS: readonly GlzTrustItem[] = [
  { text: 'Runs a tool server', detail: 'npx @acme/mcp' },
  { text: 'Runs a hook before each command', detail: './scripts/check.sh' },
  { text: 'Lets glave approve actions without asking' },
  { text: 'Turns off the sandbox', detail: null },
];

@Component({
  imports: [GlzTrustCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-trust-card
      testId="trust"
      summary="This project brings its own tools and settings"
      [items]="items"
      [answered]="answered()"
      [busy]="busy()"
      (answer)="answers.push($event)"
    />
  `,
})
class HostComponent {
  readonly items = ITEMS;
  readonly answered = signal<GlzTrustAnswer | null>(null);
  readonly busy = signal(false);
  readonly answers: GlzTrustAnswer[] = [];
}

function button(root: HTMLElement, testId: string): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
}

function text(element: Element | null | undefined): string {
  return element?.textContent.replace(/\s+/gu, ' ').trim() ?? '';
}

describe('GlzTrustCardComponent', () => {
  it('needs the person: says what it asks, lists each thing, and offers trust first', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const card = root.querySelector('glz-trust-card');
    expect(card?.getAttribute('role')).toBe('group');
    const names = (card?.getAttribute('aria-labelledby') ?? '').split(' ');
    expect(names.map((id) => text(document.getElementById(id)))).toEqual([
      'Needs you',
      'This project brings its own tools and settings',
    ]);
    const items = Array.from(root.querySelectorAll('[data-testid="trust-item"]'));
    expect(items.map(text)).toEqual([
      'Runs a tool server npx @acme/mcp',
      'Runs a hook before each command ./scripts/check.sh',
      'Lets glave approve actions without asking',
      'Turns off the sandbox',
    ]);
    expect(items[0]?.querySelector('code')?.textContent).toBe('npx @acme/mcp');
    expect(items[2]?.querySelector('code')).toBeNull();
    const buttons = Array.from(root.querySelectorAll('.actions button'));
    expect(buttons.map(text)).toEqual(['Trust this project', 'Not now']);
    expect(buttons[0]?.classList).toContain('glz-button--primary');
    await expectAccessible(root);
    button(root, 'trust-trust')?.click();
    button(root, 'trust-not-now')?.click();
    expect(fixture.componentInstance.answers).toEqual(['trust', 'not_now']);
  });

  it('waits while busy, and once answered rests and says what was answered', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(button(root, 'trust-trust')?.disabled).toBe(true);
    expect(button(root, 'trust-not-now')?.disabled).toBe(true);
    fixture.componentInstance.answered.set('trust');
    await fixture.whenStable();
    expect(button(root, 'trust-trust')).toBeNull();
    expect(text(root.querySelector('[data-testid="trust-answered"]'))).toBe('Trusted');
    const card = root.querySelector('glz-trust-card');
    expect(root.querySelector('.label')).toBeNull();
    expect(card?.classList).toContain('glz-trust-card--answered');
    expect(text(document.getElementById(card?.getAttribute('aria-labelledby') ?? ''))).toBe(
      'This project brings its own tools and settings',
    );
    fixture.componentInstance.answered.set('not_now');
    await fixture.whenStable();
    expect(text(root.querySelector('[data-testid="trust-answered"]'))).toBe('Kept off for now');
    await expectAccessible(root);
  });
});
