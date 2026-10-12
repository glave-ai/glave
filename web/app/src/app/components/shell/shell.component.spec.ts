import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { expectAccessible } from '@glave/glaze/testing';
import { ShellComponent } from './shell.component';

describe('ShellComponent', () => {
  it('has a skip link, the top bar and the main region', async () => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    const fixture = TestBed.createComponent(ShellComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="shell-skip"]')?.getAttribute('href')).toBe('#main');
    expect(root.querySelector('header glz-top-bar')).not.toBeNull();
    expect(root.querySelector('main#main')).not.toBeNull();
    await expectAccessible(root);
  });
});
