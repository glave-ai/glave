import { ChangeDetectionStrategy, Component, computed, signal, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzPopoverComponent } from '../components/popover/popover.component';
import { GlzPopoverTriggerDirective } from './popover-trigger.directive';

@Component({
  imports: [GlzPopoverComponent, GlzPopoverTriggerDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button type="button" data-testid="open" [glzPopoverTrigger]="target()">Open</button>
    <glz-popover #one testId="one"><button type="button">One</button></glz-popover>
    <glz-popover #two testId="two"><button type="button">Two</button></glz-popover>
  `,
})
class HostComponent {
  readonly second = signal(false);
  private readonly one = viewChild.required<GlzPopoverComponent>('one');
  private readonly two = viewChild.required<GlzPopoverComponent>('two');
  readonly target = computed(() => (this.second() ? this.two() : this.one()));
}

describe('GlzPopoverTriggerDirective', () => {
  it('points its button at the popover it is given, as that changes', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const button = root.querySelector<HTMLButtonElement>('[data-testid="open"]');
    const one = root.querySelector('[data-testid="one"]');
    const two = root.querySelector('[data-testid="two"]');
    expect(button?.popoverTargetElement).toBe(one);
    expect(button?.getAttribute('aria-controls')).toBe(one?.id);
    fixture.componentInstance.second.set(true);
    await fixture.whenStable();
    expect(button?.popoverTargetElement).toBe(two);
    expect(button?.getAttribute('aria-controls')).toBe(two?.id);
  });
});
