import { TestBed } from '@angular/core/testing';

import { ThemeService } from '../../services/theme.service';
import { ThemePickerComponent } from './theme-picker.component';

describe('ThemePickerComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('says the theme and moves to the next one', async () => {
    const fixture = TestBed.createComponent(ThemePickerComponent);
    await fixture.whenStable();
    const button = (fixture.nativeElement as HTMLElement).querySelector<HTMLButtonElement>(
      '[data-testid="theme-picker"]',
    );
    expect(button?.getAttribute('aria-label')).toContain('as the system');
    button?.click();
    await fixture.whenStable();
    expect(TestBed.inject(ThemeService).choice()).toBe('light');
    expect(button?.getAttribute('aria-label')).toContain('Theme: light');
  });
});
