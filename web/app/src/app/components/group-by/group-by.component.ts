import {
  ChangeDetectionStrategy,
  ChangeDetectorRef,
  Component,
  inject,
  viewChild,
} from '@angular/core';
import type { Grouping } from '@glave/client';
import { type GlzOption, GlzSegmentedComponent } from '@glave/glaze';

import { GroupingService } from '../../services/grouping.service';

/**
 * How the side nav groups the person's threads, as a switch above them: by what each is doing,
 * or by project. A choice shows at once and is kept as the person's, for every browser of
 * theirs; when it is not kept, the choice before it comes back and why shows under the switch.
 */
@Component({
  selector: 'app-group-by',
  imports: [GlzSegmentedComponent],
  templateUrl: './group-by.component.html',
  styleUrl: './group-by.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GroupByComponent {
  private readonly service = inject(GroupingService);
  private readonly changes = inject(ChangeDetectorRef);
  /** The switch, which shows the person's click at once, then what the server kept. */
  private readonly segmented = viewChild(GlzSegmentedComponent);

  protected readonly copy = { label: $localize`:@@groupBy.label:Group by` } as const;
  /** The ways the threads can be grouped, in the order the switch shows them. */
  protected readonly groupings: readonly (GlzOption & { readonly value: Grouping })[] = [
    { value: 'status', label: $localize`:@@groupBy.status:Status` },
    { value: 'project', label: $localize`:@@groupBy.project:Project` },
  ];
  protected readonly grouping = this.service.grouping;
  protected readonly problem = this.service.problem;

  protected async group(value: string | null): Promise<void> {
    const grouping = this.groupings.find((option) => option.value === value)?.value;
    if (grouping === undefined) {
      return;
    }
    await this.service.choose(grouping);
    // The switch shows what the server kept, whatever the click before it: the click's own
    // state reaches the page first, so that going back from it shows.
    this.changes.detectChanges();
    this.segmented()?.value.set(this.grouping());
  }
}
