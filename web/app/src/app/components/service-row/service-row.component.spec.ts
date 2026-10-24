import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ServiceRowComponent } from './service-row.component';

/** A list of one service, as a page holds it. */
@Component({
  imports: [ServiceRowComponent],
  template: `<div role="list">
    <app-service-row name="Google Calendar">
      <span data-testid="detail">Connected</span>
      <button type="button" appServiceAction>Disconnect</button>
    </app-service-row>
  </div>`,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
class HostComponent {}

describe('ServiceRowComponent', () => {
  it('shows its letters, its name over its line, and its action at the end', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const row = root.querySelector('app-service-row');
    expect(row?.getAttribute('role')).toBe('listitem');
    expect(row?.querySelector('.tile')?.textContent.trim()).toBe('GC');
    expect(row?.querySelector('.tile')?.getAttribute('aria-hidden')).toBe('true');
    expect(row?.querySelector('.name')?.textContent).toBe('Google Calendar');
    expect(row?.querySelector('.text [data-testid="detail"]')?.textContent).toBe('Connected');
    expect(row?.lastElementChild?.textContent).toBe('Disconnect');
    await expectAccessible(root);
  });
});
