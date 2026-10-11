import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Brain, MessagesSquare } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzRailItemComponent } from '../rail-item/rail-item.component';
import { GlzRailComponent } from './rail.component';

@Component({
  imports: [GlzRailComponent, GlzRailItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav aria-label="Main">
      <glz-rail>
        <a glzRailItem href="/chat" label="Chat" current [icon]="chat"></a>
        <a glzRailItem href="/memory" label="Memory" [icon]="memory"></a>
      </glz-rail>
    </nav>
  `,
})
class HostComponent {
  protected readonly chat = MessagesSquare;
  protected readonly memory = Brain;
}

describe('GlzRailComponent', () => {
  it('lists its destinations', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelectorAll('glz-rail a')).toHaveLength(2);
    await expectAccessible(root);
  });
});
