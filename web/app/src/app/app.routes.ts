import { inject } from '@angular/core';
import {
  type ActivatedRouteSnapshot,
  type NavigationError,
  RedirectCommand,
  Router,
  type Routes,
  type ViewTransitionInfo,
} from '@angular/router';

import { UnloadedComponent } from './components/unloaded/unloaded.component';
import { WorkspaceComponent } from './components/workspace/workspace.component';
import { adminGuard } from './guards/admin.guard';
import { hasModelGuard } from './guards/has-model.guard';
import { signedInGuard } from './guards/signed-in.guard';

/** Where a page that could not open is said to have failed, in its own place. */
const UNLOADED = '/unloaded';

/** A thread's page. */
const THREAD = 't/:thread';

/** The View Transition in which what Glaze marks `glz-crossfade` crossfades (styles/motion.css). */
const CROSSFADE = 'glz-crossfade';

/**
 * The app's pages: the workspace (chat, a project, a thread, tasks, memory, settings and the
 * pages reached from it, `settings.routes.ts`) for a signed-in person with a model set up, the
 * page that adds one, the sign-in page, and the page that says a page could not open. Each
 * title is read when its page opens, in the app's language.
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
        path: THREAD,
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
    // Admin, a room of its own for the organization's Admins and Owners.
    path: 'admin',
    canActivate: [signedInGuard, adminGuard],
    loadComponent: () => import('./components/admin/admin.component').then((m) => m.AdminComponent),
    children: [
      {
        path: '',
        pathMatch: 'full',
        title: () => $localize`:@@routes.people:People · Admin · glave`,
        loadComponent: () =>
          import('./components/people/people.component').then((m) => m.PeopleComponent),
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
  {
    // Part of the app, not a page anyone links to: what stands in for one that could not open
    // (`unloadedPage`). Its code is in the first load, and it needs no sign-in, so it opens
    // whatever could not.
    path: UNLOADED.slice(1),
    title: 'glave',
    component: UnloadedComponent,
    data: { page: true },
  },
  { path: '**', redirectTo: '' },
];

/**
 * A page that could not open, as when its code could not load (glave was updated while the tab
 * was open, or the network dropped): the page that says so opens in its place. The address and
 * the tab's history are left as they were, the page's own when it was the first to open, so a
 * reload, the app's or the browser's, opens it again with the code glave has now. (Showing the
 * page's address instead, with `browserUrl`, would keep the page that says so in the history
 * entry, and a reload would open that again.)
 */
export function unloadedPage(error: NavigationError): RedirectCommand | undefined {
  const instead = inject(Router).parseUrl(UNLOADED);
  // Only ever once: should the page that says so not open either, nothing more is tried.
  return error.url === UNLOADED
    ? undefined
    : new RedirectCommand(instead, { skipLocationChange: true });
}

/** The thread whose page `route` opens, if it opens one. */
function threadOf(route: ActivatedRouteSnapshot): string | null {
  let page = route;
  while (page.firstChild !== null) {
    page = page.firstChild;
  }
  return page.routeConfig?.path === THREAD ? page.paramMap.get('thread') : null;
}

/**
 * Crossfades the workspace's pane (the element marked `glz-crossfade`) as the person switches
 * from one thread to another; every other change of page, and one in a browser that cannot
 * crossfade, shows the next page at once.
 */
export function crossfadeThreads({ transition, from, to }: ViewTransitionInfo): void {
  const before = threadOf(from);
  const after = threadOf(to);
  if (before !== null && after !== null && before !== after && 'types' in transition) {
    transition.types.add(CROSSFADE);
  }
}
