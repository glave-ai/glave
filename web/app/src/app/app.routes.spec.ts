import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter, withViewTransitions } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { crossfadeThreads } from './app.routes';

@Component({ template: '', changeDetection: ChangeDetectionStrategy.OnPush })
class PageComponent {}

/**
 * The types of each View Transition the router starts, in order, from a stand-in for the
 * browser's, which jsdom lacks: it runs the update and is done.
 */
function transitions(): Set<string>[] {
  const started: Set<string>[] = [];
  Object.assign(document, {
    startViewTransition: (update: () => Promise<void>) => {
      const types = new Set<string>();
      started.push(types);
      const done = update();
      return {
        types,
        ready: done,
        finished: done,
        updateCallbackDone: done,
        skipTransition: () => undefined,
      };
    },
  });
  return started;
}

describe('crossfadeThreads', () => {
  afterEach(() => {
    Reflect.deleteProperty(document, 'startViewTransition');
  });

  it('crossfades the pane from one thread to another, and nothing else', async () => {
    const started = transitions();
    TestBed.configureTestingModule({
      providers: [
        provideRouter(
          [
            { path: 't/:thread', component: PageComponent },
            { path: 'tasks', component: PageComponent },
          ],
          withViewTransitions({
            skipInitialTransition: true,
            onViewTransitionCreated: crossfadeThreads,
          }),
        ),
      ],
    });
    const harness = await RouterTestingHarness.create('/t/one');
    await harness.navigateByUrl('/t/two');
    await harness.navigateByUrl('/tasks');
    await harness.navigateByUrl('/t/two');
    await harness.navigateByUrl('/t/two?again');
    const types = started.map((each) => [...each]);
    expect(types).toEqual([['glz-crossfade'], [], [], []]);
  });

  it('shows the next thread at once where the browser cannot crossfade', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter(
          [{ path: 't/:thread', component: PageComponent }],
          withViewTransitions({
            skipInitialTransition: true,
            onViewTransitionCreated: crossfadeThreads,
          }),
        ),
      ],
    });
    const harness = await RouterTestingHarness.create('/t/one');
    await harness.navigateByUrl('/t/two');
    expect(TestBed.inject(Router).url).toBe('/t/two');
  });
});
