import { LiveAnnouncer } from '@angular/cdk/a11y';
import type { Provider } from '@angular/core';

/** A tab a page opens for a sign-in, standing in for the browser's. */
export interface FakeTab {
  readonly tab: Window;
  /** Where the page sent it. */
  readonly sentTo: string[];
  /** Whether the page closed it. */
  readonly closed: () => boolean;
}

/** Makes `window.open` give a fake tab, which this hands back. */
export function fakeTab(): FakeTab {
  const sentTo: string[] = [];
  let closed = false;
  const tab = {
    opener: window,
    get closed() {
      return closed;
    },
    close: () => {
      closed = true;
    },
    location: { replace: (url: string) => sentTo.push(url) },
  } as unknown as Window;
  vi.spyOn(window, 'open').mockReturnValue(tab);
  return { tab, sentTo, closed: () => closed };
}

/** Polls the fake answers while a sign-in waits: a tick each, the flow `f1` again. */
export async function stillWaiting(): Promise<object> {
  await new Promise((resolve) => setTimeout(resolve));
  return { flow: 'f1' };
}

/** What a page says to screen readers, and the provider that collects it. */
export function announced(): { readonly said: string[]; readonly provider: Provider } {
  const said: string[] = [];
  const announce = (text: string): Promise<void> => {
    said.push(text);
    return Promise.resolve();
  };
  return { said, provider: { provide: LiveAnnouncer, useValue: { announce } } };
}
