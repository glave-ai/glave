import { Service, signal } from '@angular/core';

/**
 * Whether the side nav shows: beside the page on wide screens, as a drawer on narrow ones.
 * The workspace's layout binds it; each page's header toggles it.
 */
@Service()
export class NavService {
  readonly open = signal(true);

  toggle(): void {
    this.open.update((open) => !open);
  }
}
