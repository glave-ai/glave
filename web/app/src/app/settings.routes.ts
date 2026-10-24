import type { Routes } from '@angular/router';

/**
 * Settings' pages: Settings itself, and the pages reached from it, each loaded when it opens.
 * Loaded with the first of them, so a page added here adds nothing to the app's first load.
 */
export const SETTINGS_ROUTES: Routes = [
  {
    path: '',
    pathMatch: 'full',
    title: () => $localize`:@@routes.settings:Settings · glave`,
    loadComponent: () =>
      import('./components/settings/settings.component').then((m) => m.SettingsComponent),
  },
  {
    path: 'connections',
    title: () => $localize`:@@routes.connections:Connections · glave`,
    loadComponent: () =>
      import('./components/connections/connections.component').then((m) => m.ConnectionsComponent),
  },
];
