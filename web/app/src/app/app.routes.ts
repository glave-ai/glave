import type { Routes } from '@angular/router';

import { WorkspaceComponent } from './components/workspace/workspace.component';
import { signedInGuard } from './guards/signed-in.guard';

/**
 * The app's pages: the workspace (chat, a thread, tasks, memory, settings) for a signed-in
 * person, and the sign-in page.
 */
export const routes: Routes = [
  {
    path: '',
    component: WorkspaceComponent,
    canActivate: [signedInGuard],
    children: [
      {
        path: '',
        pathMatch: 'full',
        title: 'glave',
        loadComponent: () =>
          import('./components/home/home.component').then((m) => m.HomeComponent),
      },
      {
        path: 't/:thread',
        title: 'Thread · glave',
        loadComponent: () =>
          import('./components/thread/thread.component').then((m) => m.ThreadComponent),
      },
      {
        path: 'tasks',
        title: 'Tasks · glave',
        loadComponent: () =>
          import('./components/tasks/tasks.component').then((m) => m.TasksComponent),
      },
      {
        path: 'memory',
        title: 'Memory · glave',
        loadComponent: () =>
          import('./components/memory/memory.component').then((m) => m.MemoryComponent),
      },
      {
        path: 'settings',
        title: 'Settings · glave',
        loadComponent: () =>
          import('./components/settings/settings.component').then((m) => m.SettingsComponent),
      },
    ],
  },
  {
    path: 'sign-in',
    title: 'Sign in · glave',
    loadComponent: () =>
      import('./components/sign-in/sign-in.component').then((m) => m.SignInComponent),
  },
  { path: '**', redirectTo: '' },
];
