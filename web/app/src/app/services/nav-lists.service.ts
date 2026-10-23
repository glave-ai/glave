import { DOCUMENT } from '@angular/common';
import { Service, computed, effect, inject, signal } from '@angular/core';

import { toNavLists, withList } from '../lib/nav-lists';
import { readStored, storageOf, writeStored } from '../lib/storage';

const KEY = 'glave-nav-lists';

/**
 * How the person left the side nav's lists: which headings they closed (Projects, Chats, a
 * project). Kept in this browser's storage when it allows; without it they last for the page.
 */
@Service()
export class NavListsService {
  private readonly storage = storageOf(inject(DOCUMENT));
  private readonly lists = signal(toNavLists(readStored(this.storage, KEY)));

  /** The lists closed, by key (`PROJECTS_LIST`, `CHATS_LIST` or a project's id). */
  readonly collapsed = computed<ReadonlySet<string>>(() => new Set(this.lists().collapsed));

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, this.lists());
    });
  }

  /** Opens or closes the list `list`. */
  setExpanded(list: string, expanded: boolean): void {
    this.lists.update((lists) => ({ collapsed: withList(lists.collapsed, list, !expanded) }));
  }
}
