import { Clipboard } from '@angular/cdk/clipboard';
import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzCopyDirective } from './copy.directive';

@Component({
  imports: [GlzCopyDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<button type="button" glzCopy="cargo test" (copied)="results.push($event)">
    Copy
  </button>`,
})
class HostComponent {
  readonly results: boolean[] = [];
}

describe('GlzCopyDirective', () => {
  it('copies its text on click and says whether it worked', async () => {
    const copy = vi.fn(() => true);
    TestBed.configureTestingModule({ providers: [{ provide: Clipboard, useValue: { copy } }] });
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    (fixture.nativeElement as HTMLElement).querySelector('button')?.click();
    expect(copy).toHaveBeenCalledWith('cargo test');
    expect(fixture.componentInstance.results).toEqual([true]);
  });
});
