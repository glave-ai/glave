import { DOCUMENT } from '@angular/common';
import { Service, computed, effect, inject, signal } from '@angular/core';

import type { NavSort } from '../lib/nav';
import { toNavLists, withList } from '../lib/nav-lists';
import { readStored, storageOf, writeStored } from '../lib/storage';

const KEY = 'glave-nav-lists';

/**
 * How the person left the side nav's lists: which headings they closed (Projects, Chats, a
 * project), and which lists they sort by name instead of by recent activity. Kept in this
 * browser's storage when it allows; without it they last for the page.
 */
@Service()
export class NavListsService {
  private readonly storage = storageOf(inject(DOCUMENT), 'localStorage');
  private readonly lists = signal(toNavLists(readStored(this.storage, KEY)));

  /** The lists closed, by key (`PROJECTS_LIST`, `CHATS_LIST` or a project's id). */
  readonly collapsed = computed<ReadonlySet<string>>(() => new Set(this.lists().collapsed));
  /** The lists sorted by name, by key. */
  readonly byName = computed<ReadonlySet<string>>(() => new Set(this.lists().byName));

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, this.lists());
    });
  }

  /** Opens or closes the list `list`. */
  setExpanded(list: string, expanded: boolean): void {
    this.lists.update((lists) => ({
      ...lists,
      collapsed: withList(lists.collapsed, list, !expanded),
    }));
  }

  /** Sorts the list `list` by `sort`. */
  setSort(list: string, sort: NavSort): void {
    this.lists.update((lists) => ({
      ...lists,
      byName: withList(lists.byName, list, sort === 'name'),
    }));
  }
}
