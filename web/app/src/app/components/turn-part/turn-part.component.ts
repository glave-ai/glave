import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { GlzStepsComponent } from '@glave/glaze';

import type { HeldImage, ThreadItem } from '../../lib/conversation';
import type { TurnPart } from '../../lib/runs';
import { ReplySourcesComponent } from '../reply-sources/reply-sources.component';
import { ThreadImageComponent } from '../thread-image/thread-image.component';
import { ThreadItemComponent } from '../thread-item/thread-item.component';

/** An image a call's result holds, with a key of its own: one image may show twice. */
interface ShownImage {
  readonly key: string;
  readonly image: HeldImage;
}

/** The images the calls among `items` hold, in order. */
function imagesIn(items: readonly ThreadItem[]): readonly ShownImage[] {
  return items.flatMap((item) =>
    item.kind === 'call'
      ? item.images.map((image, at) => ({ key: `${item.key}/${String(at)}`, image }))
      : [],
  );
}

/**
 * One part of a turn, in its place: a run of calls as the server's line, which opens to its
 * calls, the line's room kept, unseen, until the server has told it, and under it, always in
 * view, the images its calls made; or anything else as its item, a reply with the pages it
 * drew on from the web under it.
 */
@Component({
  selector: 'app-turn-part',
  imports: [GlzStepsComponent, ReplySourcesComponent, ThreadImageComponent, ThreadItemComponent],
  templateUrl: './turn-part.component.html',
  styleUrl: './turn-part.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnPartComponent {
  readonly part = input.required<TurnPart>();

  protected readonly copy = {
    /** What names a run for screen readers until the server has told it. */
    steps: $localize`:@@turnPart.steps:Steps`,
  } as const;
  /** A run's line: the server's words, else a stand-in only screen readers read, and which. */
  protected readonly line = computed(() => {
    const part = this.part();
    const sentence = part.kind === 'run' ? part.sentence : null;
    return { words: sentence ?? this.copy.steps, pending: sentence === null };
  });
  /** The images its calls made, shown under it whether or not it is open. */
  protected readonly images = computed(() => {
    const part = this.part();
    return imagesIn(part.kind === 'run' ? part.items : [part.item]);
  });
}
