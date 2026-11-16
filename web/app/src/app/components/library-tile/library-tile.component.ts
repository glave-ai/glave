import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import type { ShownItem } from '../../lib/library';
import { FilesService, drawn } from '../../services/files.service';

/** How far beyond the view a tile's picture is read ahead, so it is there as it scrolls in. */
const AHEAD = '200px';

/**
 * One item of the Library's grid: its picture, which opens it, its name, when it was made and
 * the chat it came from, a link there while that chat is. Its picture is read through the server
 * only while the tile is in view or near it, and let go of once it scrolls away, so a Library of
 * hundreds reads only what the person comes to see.
 */
@Component({
  selector: 'app-library-tile',
  imports: [RouterLink],
  templateUrl: './library-tile.component.html',
  styleUrl: './library-tile.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LibraryTileComponent {
  readonly shown = input.required<ShownItem>();
  /** Emitted when the person opens the item. */
  readonly opened = output();

  private readonly files = inject(FilesService);
  /** Whether the tile is in view or near it: always, where the browser can't say. */
  private readonly near = signal(!('IntersectionObserver' in globalThis));
  /**
   * The file its picture is, by the item's project, id and kind of file: the same however often
   * the Library hands it a new copy of the item, so the picture is never let go of and asked for
   * again, as when another item is deleted.
   */
  private readonly picture = computed(
    () => {
      const { project, asset, media_type: mediaType } = this.shown().item;
      return { project, asset, mediaType };
    },
    {
      equal: (one, other) =>
        one.project === other.project &&
        one.asset === other.asset &&
        one.mediaType === other.mediaType,
    },
  );
  private readonly file = drawn(() => {
    if (!this.near()) {
      return null;
    }
    const { project, asset, mediaType } = this.picture();
    return this.files.holdAsset(project, asset, mediaType);
  });
  protected readonly url = this.file.url;

  constructor() {
    if (!('IntersectionObserver' in globalThis)) {
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        const latest = entries.at(-1);
        if (latest !== undefined) {
          this.near.set(latest.isIntersecting);
        }
      },
      { rootMargin: AHEAD },
    );
    observer.observe(inject<ElementRef<HTMLElement>>(ElementRef).nativeElement);
    inject(DestroyRef).onDestroy(() => {
      observer.disconnect();
    });
  }

  protected open(): void {
    this.opened.emit();
  }
}
