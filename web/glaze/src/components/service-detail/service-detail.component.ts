import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { ChevronRight } from 'lucide';

import { INERT_UNTIL_BOUND } from '../../lib/inert-until-bound';
import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzMarkdownComponent } from '../markdown/markdown.component';

/** One file that comes with what is added. */
export interface GlzServiceDetailFile {
  /** Where it goes: `scripts/merged.sh`. */
  readonly path: string;
  /** Its text, drawn as plain code. */
  readonly text: string;
  /** Whether it is a program that runs, so it is marked. */
  readonly runsCode: boolean;
}

/** What a question adds, for the person to read before they answer. */
export interface GlzServiceDetail {
  /** Its header lines (a skill's frontmatter), as written; empty for none. */
  readonly front: string;
  /** Its text, markdown. */
  readonly text: string;
  readonly files: readonly GlzServiceDetailFile[];
  /** What was cut to show it, in the server's sentence; `null` when nothing was. */
  readonly cut: string | null;
}

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  summary: 'summary',
  front: 'front',
  text: 'text',
  file: 'file',
  runs: 'runs',
  code: 'code',
  cut: 'cut',
} as const;

/**
 * What a card's question adds, folded shut under it: "What it adds", opened, shows its header
 * lines, its text drawn as markdown, and the files with it, a program marked as one, each
 * opening as plain code. However much it holds, it stays at most half the screen tall: the
 * text, the list of files and an opened file each scroll on their own, an opened file both ways,
 * never wrapping, and a fade marks each end with more past it.
 *
 * ```html
 * <glz-service-detail [detail]="adds" testId="install-adds" />
 * ```
 */
@Component({
  selector: 'glz-service-detail',
  imports: [GlzIconComponent, GlzMarkdownComponent],
  templateUrl: './service-detail.component.html',
  styleUrl: './service-detail.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { ...INERT_UNTIL_BOUND, '[attr.data-testid]': 'testId()' },
})
export class GlzServiceDetailComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('serviceDetail', {
    label: 'What it adds',
    textLabel: 'Its text',
    filesLabel: 'Files with it',
    runsCodeLabel: 'runs code',
  });

  readonly detail = input.required<GlzServiceDetail>();
  /** What the fold says while shut. */
  readonly label = input(this.words.label);
  /** What names its text's scrolling region to a screen reader. */
  readonly textLabel = input(this.words.textLabel);
  /** The heading over the files. */
  readonly filesLabel = input(this.words.filesLabel);
  /** What marks a file that is a program. */
  readonly runsCodeLabel = input(this.words.runsCodeLabel);

  protected readonly chevron = ChevronRight;
  protected readonly front = computed(() => this.detail().front);
  protected readonly text = computed(() => this.detail().text);
  protected readonly files = computed(() => this.detail().files);
  protected readonly hasFiles = computed(() => this.files().length > 0);
  protected readonly cut = computed(() => this.detail().cut);
}
