import type { Routes } from '@angular/router';

import { WorkspaceComponent } from './components/workspace/workspace.component';
import { hasModelGuard } from './guards/has-model.guard';
import { signedInGuard } from './guards/signed-in.guard';

/**
 * The app's pages: the workspace (chat, a project, a thread, tasks, memory, settings and the
 * pages reached from it, `settings.routes.ts`) for a signed-in person with a model set up, the
 * page that adds one, and the sign-in page. Each title is read when its page opens, in the
 * app's language.
 */
export const routes: Routes = [
  {
    path: '',
    component: WorkspaceComponent,
    canActivate: [signedInGuard, hasModelGuard],
    children: [
      {
        path: '',
        pathMatch: 'full',
        title: 'glave',
        loadComponent: () =>
          import('./components/home/home.component').then((m) => m.HomeComponent),
      },
      {
        // A project's page, where glave run in its folder opens: a new thread starts in it.
        path: 'p/:project',
        title: () => $localize`:@@routes.project:Project · glave`,
        loadComponent: () =>
          import('./components/home/home.component').then((m) => m.HomeComponent),
      },
      {
        path: 't/:thread',
        title: () => $localize`:@@routes.thread:Thread · glave`,
        loadComponent: () =>
          import('./components/thread/thread.component').then((m) => m.ThreadComponent),
      },
      {
        path: 'tasks',
        title: () => $localize`:@@routes.tasks:Tasks · glave`,
        loadComponent: () =>
          import('./components/tasks/tasks.component').then((m) => m.TasksComponent),
      },
      {
        path: 'memory',
        title: () => $localize`:@@routes.memory:Memory · glave`,
        loadComponent: () =>
          import('./components/memory/memory.component').then((m) => m.MemoryComponent),
      },
      {
        path: 'settings',
        loadChildren: () => import('./settings.routes').then((m) => m.SETTINGS_ROUTES),
      },
    ],
  },
  {
    path: 'connect',
    title: () => $localize`:@@routes.connect:Add a model · glave`,
    canActivate: [signedInGuard],
    loadComponent: () =>
      import('./components/connect/connect.component').then((m) => m.ConnectComponent),
  },
  {
    path: 'sign-in',
    title: () => $localize`:@@routes.signIn:Sign in · glave`,
    loadComponent: () =>
      import('./components/sign-in/sign-in.component').then((m) => m.SignInComponent),
  },
  { path: '**', redirectTo: '' },
];
