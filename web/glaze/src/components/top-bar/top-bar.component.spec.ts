import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzTopBarComponent } from './top-bar.component';

@Component({
  imports: [GlzTopBarComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <header>
      <glz-top-bar>
        <button glzTopBarStart type="button">Menu</button>
        <span>Atlas</span>
        <button glzTopBarEnd type="button">Theme</button>
      </glz-top-bar>
    </header>
  `,
})
class HostComponent {}

describe('GlzTopBarComponent', () => {
  it('puts each slot in its place', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('.start')?.textContent).toBe('Menu');
    expect(root.querySelector('.title')?.textContent).toBe('Atlas');
    expect(root.querySelector('.end')?.textContent).toBe('Theme');
    await expectAccessible(root);
  });
});
