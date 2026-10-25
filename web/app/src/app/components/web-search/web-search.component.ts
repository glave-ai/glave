import {
  ChangeDetectionStrategy,
  ChangeDetectorRef,
  Component,
  computed,
  inject,
  viewChild,
} from '@angular/core';
import { GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent } from '@glave/glaze';

import { SearchService } from '../../services/search.service';

/**
 * In Settings: whether glave may search the web for the person, a switch, with the server's
 * line saying who searches and what it costs, and, while it is on, how often it searched this
 * month. Where it is not the person's to choose, nothing, not even room in the page.
 */
@Component({
  selector: 'app-web-search',
  imports: [GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent],
  templateUrl: './web-search.component.html',
  styleUrl: './web-search.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class WebSearchComponent {
  protected readonly service = inject(SearchService);
  private readonly changes = inject(ChangeDetectorRef);
  protected readonly copy = {
    heading: $localize`:@@webSearch.heading:Web search`,
    label: $localize`:@@webSearch.label:When it helps`,
    search: $localize`:@@webSearch.search:Let glave search the web`,
  } as const;
  /** The server's word on it, while it is the person's to choose. */
  private readonly chosen = computed(() => {
    const search = this.service.search();
    return search?.kind === 'chosen' ? search : null;
  });
  protected readonly shown = computed(() => this.chosen() !== null);
  protected readonly on = this.service.on;
  protected readonly detail = computed(() => this.chosen()?.detail ?? '');
  protected readonly usage = computed(() => this.chosen()?.usage?.sentence ?? null);
  protected readonly problem = this.service.problem;
  /** The switch, which shows the person's click at once, then what the server says. */
  private readonly box = viewChild(GlzCheckboxComponent);

  constructor() {
    void this.service.load();
  }

  /** Turns web search on or off; the server's answer is what stays. */
  protected async set(on: boolean): Promise<void> {
    await this.service.set(on);
    // The switch shows what the server said, whatever the click before it.
    this.changes.detectChanges();
    this.box()?.checked.set(this.on());
  }
}
