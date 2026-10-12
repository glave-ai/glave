import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { Router } from '@angular/router';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzFormFieldComponent,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';
import { ThemeService, isChoice } from '../../services/theme.service';
import { PageHeaderComponent } from '../page-header/page-header.component';

const COPY = {
  heading: 'Settings',
  appearance: 'Appearance',
  theme: 'Theme',
  system: 'System',
  light: 'Light',
  dark: 'Dark',
  policy: 'Settings and policy',
  policyText:
    'Settings come from config.toml files and flags, nearest first; policy only narrows them. Changing them here comes later. To see every setting and where it came from, run this in your project:',
  policyCommand: 'glave policy explain',
  server: 'This server',
  serverUnknown: 'Reading…',
  signOut: 'Sign out of this tab',
} as const;

/** Settings, read-only for now: the theme, where settings come from, the server, sign out. */
@Component({
  selector: 'app-settings',
  imports: [
    PageHeaderComponent,
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
  protected readonly themes = [
    { value: 'system', label: COPY.system },
    { value: 'light', label: COPY.light },
    { value: 'dark', label: COPY.dark },
  ] as const;
  private readonly theme = inject(ThemeService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly info = inject(ServerService).info;

  protected readonly choice = this.theme.choice;
  protected readonly server = computed(() => {
    const info = this.info();
    return info === null
      ? COPY.serverUnknown
      : `${info.name} ${info.version} · protocol ${info.protocols.join(', ')} · approvals: ${info.approval}`;
  });

  constructor() {
    void inject(ServerService).load();
  }

  protected pick(choice: string | null): void {
    if (isChoice(choice)) {
      this.theme.set(choice);
    }
  }

  protected async signOut(): Promise<void> {
    this.session.signOut();
    await this.router.navigateByUrl('/sign-in');
  }
}
