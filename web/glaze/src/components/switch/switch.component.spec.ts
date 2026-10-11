import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FormControl, ReactiveFormsModule } from '@angular/forms';

import { expectAccessible } from '../../testing/axe';
import { GlzSwitchComponent } from './switch.component';

@Component({
  imports: [GlzSwitchComponent, ReactiveFormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-switch testId="switch" [(checked)]="cloud">Run in the cloud</glz-switch>
    <glz-switch testId="switch" [formControl]="memory">Memory</glz-switch>
  `,
})
class HostComponent {
  readonly cloud = signal(false);
  readonly memory = new FormControl(true);
}

describe('GlzSwitchComponent', () => {
  it('is a labelled switch bound two ways or to a form', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [cloud, memory] = Array.from(
      root.querySelectorAll<HTMLInputElement>('[data-testid="switch-input"]'),
    );
    expect(cloud?.getAttribute('role')).toBe('switch');
    expect(cloud?.getAttribute('aria-checked')).toBe('false');
    cloud?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.cloud()).toBe(true);
    expect(cloud?.getAttribute('aria-checked')).toBe('true');
    expect(memory?.checked).toBe(true);
    memory?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.memory.value).toBe(false);
    fixture.componentInstance.memory.disable();
    await fixture.whenStable();
    expect(memory?.disabled).toBe(true);
    await expectAccessible(root);
  });
});
