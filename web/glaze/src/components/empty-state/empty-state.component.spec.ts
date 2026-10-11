import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { MessagesSquare } from 'lucide';

import { expectAccessible } from '../../testing/axe';
import { GlzEmptyStateComponent } from './empty-state.component';

@Component({
  imports: [GlzEmptyStateComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-empty-state heading="No threads yet" [icon]="icon">
      Start one and it shows up here.
      <button glzEmptyStateAction type="button">New chat</button>
    </glz-empty-state>
  `,
})
class HostComponent {
  protected readonly icon = MessagesSquare;
}

describe('GlzEmptyStateComponent', () => {
  it('shows a heading, the text and the action', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('.heading')?.textContent).toBe('No threads yet');
    expect(root.querySelector('.text')?.textContent).toContain('Start one');
    expect(root.querySelector('.actions button')?.textContent).toBe('New chat');
    expect(root.querySelector('glz-icon svg')).not.toBeNull();
    await expectAccessible(root);
  });
});
