import type { Routes } from '@angular/router';

import { ShellComponent } from './components/shell/shell.component';

/** The app's pages. The chat screens arrive with the session client; Glaze's showcase is here now. */
export const routes: Routes = [
  {
    path: '',
    component: ShellComponent,
    children: [
      {
        path: 'glaze',
        title: 'Glaze',
        loadComponent: () =>
          import('./components/showcase/showcase.component').then((m) => m.ShowcaseComponent),
      },
      { path: '', pathMatch: 'full', redirectTo: 'glaze' },
    ],
  },
  { path: '**', redirectTo: '' },
];
