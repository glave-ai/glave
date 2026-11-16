import { ChangeDetectionStrategy, Component, inject, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';

import type { ShownItem } from '../../lib/library';
import { FilesService, drawn } from '../../services/files.service';

/**
 * One item of the Library's grid: its picture, which opens it, its name, when it was made and
 * the chat it came from, a link there while that chat is. Its picture is held while it is
 * drawn, read once through the server.
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
  private readonly file = drawn(() => {
    const { item } = this.shown();
    return this.files.holdAsset(item.asset, item.media_type);
  });
  protected readonly url = this.file.url;

  protected open(): void {
    this.opened.emit();
  }
}
