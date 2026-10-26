import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import type { ReplySources } from '@glave/client';
import { injectId } from '@glave/glaze';

import { sourceLinks } from '../../lib/sources';

/**
 * What a reply drew on from the web, under it: the server's sentence ("Sources: 2 pages"), then
 * a link to each page, its title and, muted beside it, where it is. Each opens in a new tab,
 * and says so to screen readers.
 */
@Component({
  selector: 'app-reply-sources',
  templateUrl: './reply-sources.component.html',
  styleUrl: './reply-sources.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ReplySourcesComponent {
  readonly sources = input.required<ReplySources>();

  protected readonly copy = {
    newTab: $localize`:@@replySources.newTab:(opens in a new tab)`,
  } as const;
  protected readonly headingId = injectId('app-reply-sources');
  protected readonly sentence = computed(() => this.sources().sentence);
  protected readonly links = computed(() => sourceLinks(this.sources()));
}
