import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import type { ServerInfo } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzFormFieldComponent,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { SessionService } from '../../services/session.service';
import { type ThemeChoice, ThemeService } from '../../services/theme.service';

const COPY = {
  heading: 'Settings',
  appearance: 'Appearance',
  theme: 'Theme',
  system: 'As the system',
  light: 'Light',
  dark: 'Evening',
  policy: 'Settings and policy',
  policyText:
    'Settings come from config.toml files and flags, nearest first; policy only narrows them. Changing them here comes later. To see every setting and where it came from, run this in your project:',
  policyCommand: 'glave policy explain',
  server: 'This server',
  serverUnknown: 'Reading…',
  signOut: 'Sign out of this tab',
} as const;

function isTheme(value: string | null): value is ThemeChoice {
  return value === 'system' || value === 'light' || value === 'dark';
}

/** Settings, read-only for now: the theme, where settings come from, the server, sign out. */
@Component({
  selector: 'app-settings',
  imports: [
    GlzButtonComponent,
    GlzCardComponent,
    GlzFormFieldComponent,
    GlzRadioComponent,
    GlzRadioGroupComponent,
  ],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SettingsComponent {
  protected readonly copy = COPY;
  private readonly theme = inject(ThemeService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly info = signal<ServerInfo | null>(null);

  protected readonly choice = this.theme.choice;
  protected readonly server = computed(() => {
    const info = this.info();
    return info === null
      ? COPY.serverUnknown
      : `${info.name} ${info.version} · protocol ${info.protocols.join(', ')}`;
  });

  constructor() {
    void this.session.call('server/discover', {}).then(
      (info) => {
        this.info.set(info);
      },
      () => undefined,
    );
  }

  protected pick(choice: string | null): void {
    if (isTheme(choice)) {
      this.theme.set(choice);
    }
  }

  protected async signOut(): Promise<void> {
    this.session.signOut();
    await this.router.navigateByUrl('/sign-in');
  }
}
