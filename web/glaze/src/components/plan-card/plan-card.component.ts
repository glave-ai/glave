import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  afterNextRender,
  booleanAttribute,
  computed,
  inject,
  input,
  output,
  signal,
  viewChild,
  viewChildren,
} from '@angular/core';
import { Check, Circle, LoaderCircle, Plus, X } from 'lucide';

import { injectId } from '../../tokens/ids';
import type { GlzIconData } from '../../lib/icons';
import { glzText } from '../../lib/words';
import { testIds } from '../../lib/test-ids';
import { GlzButtonComponent } from '../button/button.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzInputDirective } from '../../directives/input.directive';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  step: 'step',
  build: 'build',
  change: 'change',
  edit: 'edit',
  input: 'input',
  remove: 'remove',
  add: 'add',
  save: 'save',
  cancel: 'cancel',
} as const;

/** How far a step has come: to do, doing, done. */
export type GlzPlanStatus = 'pending' | 'in_progress' | 'completed';

/** One step of a plan: what to do, and how far it has come. */
export interface GlzPlanStep {
  readonly step: string;
  readonly status: GlzPlanStatus;
}

/** The words for each status, as screen readers hear a step's mark. */
export type GlzPlanStatusLabels = Readonly<Record<GlzPlanStatus, string>>;

/** Where the keyboard goes once the card has drawn a change: a step's field, by its place. */
type Focus = number | 'add' | 'summary';

/** Each status's mark. */
const MARKS: Readonly<Record<GlzPlanStatus, GlzIconData>> = {
  pending: Circle,
  in_progress: LoaderCircle,
  completed: Check,
};

/**
 * A plan, as a card: its sentence (`summary`, the app's or a server's), then its steps in
 * order, each with a mark for how far it has come (a ring to do, a turning ring doing, a check
 * done), so steps tick off as the work goes on. A `proposed` plan needs the person, as
 * `glz-approval-card` does: a "Needs you" label, then Build it (the primary), Change it and
 * Edit. `size="sm"` makes it tighter, for a column of facts beside a page. Editing puts each
 * step in a field of its own, to change, remove or add to, then Save,
 * which emits the steps written (each trimmed, the blank left out), or Cancel. Change it
 * (`revise`) only says the person wants changes; the app takes their words. The keyboard
 * follows: into the first field on Edit, into a field added, to the next field when one is
 * removed, and back to the sentence once the edit is saved or cancelled.
 *
 * ```html
 * <glz-plan-card [summary]="plan.sentence" [steps]="plan.steps" [proposed]="plan.proposed"
 *   (build)="build()" (revise)="revise()" (edit)="edit($event)" />
 * ```
 */
@Component({
  selector: 'glz-plan-card',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent, GlzInputDirective],
  templateUrl: './plan-card.component.html',
  styleUrl: './plan-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-plan-card--proposed]': 'proposed()',
    '[class.glz-plan-card--sm]': 'small()',
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
    '[attr.aria-busy]': 'ariaBusy()',
  },
})
export class GlzPlanCardComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('planCard', {
    label: 'Needs you',
    buildLabel: 'Build it',
    changeLabel: 'Change it',
    editLabel: 'Edit',
    stepLabel: 'Step',
    removeLabel: 'Remove step',
    addLabel: 'Add a step',
    saveLabel: 'Save',
    cancelLabel: 'Cancel',
    statusLabels: { pending: 'To do', in_progress: 'Doing', completed: 'Done' },
  });

  /** What the plan is, in one sentence. */
  readonly summary = input.required<string>();
  /** Its steps, in order. */
  readonly steps = input.required<readonly GlzPlanStep[]>();
  /** Whether it waits for the person to build it, edit it or ask for changes. */
  readonly proposed = input(false, { transform: booleanAttribute });
  /** `sm`: a tighter card, its sentence a section's heading, for a column of facts beside a page. */
  readonly size = input<'md' | 'sm'>('md');
  /** Whether what the person did is being sent, so the buttons wait. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** The label over a plan that waits. */
  readonly label = input(this.words.label);
  readonly buildLabel = input(this.words.buildLabel);
  readonly changeLabel = input(this.words.changeLabel);
  readonly editLabel = input(this.words.editLabel);
  /** What a step's field is called, its place in the list after it: "Step 2". */
  readonly stepLabel = input(this.words.stepLabel);
  /** What a step's remove button is called, its step's field describing it. */
  readonly removeLabel = input(this.words.removeLabel);
  readonly addLabel = input(this.words.addLabel);
  readonly saveLabel = input(this.words.saveLabel);
  readonly cancelLabel = input(this.words.cancelLabel);
  readonly statusLabels = input<GlzPlanStatusLabels>(this.words.statusLabels);
  /** The person builds the plan. */
  readonly build = output();
  /** The person wants it changed: the app takes their words. */
  readonly revise = output();
  /** The person's steps in place of the plan's, each trimmed, none blank. */
  readonly edit = output<readonly string[]>();

  protected readonly addIcon = Plus;
  protected readonly removeIcon = X;
  protected readonly labelId = injectId('glz-plan-label');
  protected readonly summaryId = injectId('glz-plan-summary');
  private readonly fieldId = injectId('glz-plan-step');
  private readonly injector = inject(Injector);
  private readonly summaryText = viewChild.required('summaryText', { read: ElementRef });
  private readonly stepFields = viewChildren('stepField', { read: ElementRef });
  private readonly addButton = viewChild('addButton', { read: ElementRef });
  /** The steps being written, while the person edits them; `null` otherwise. */
  private readonly draft = signal<readonly string[] | null>(null);
  protected readonly editing = computed(() => this.proposed() && this.draft() !== null);
  protected readonly waits = computed(() => this.proposed() && !this.editing());
  protected readonly ariaBusy = computed(() => (this.busy() ? 'true' : null));
  protected readonly small = computed(() => this.size() === 'sm');
  /** Named by its label and sentence while it waits, by its sentence otherwise. */
  protected readonly labelledBy = computed(() =>
    this.waits() ? `${this.labelId} ${this.summaryId}` : this.summaryId,
  );
  /** Each step as it shows: its words, and its mark with what the mark means. */
  protected readonly shown = computed(() =>
    this.steps().map((step) => ({
      step: step.step,
      icon: MARKS[step.status],
      mark: this.statusLabels()[step.status],
      doing: step.status === 'in_progress',
      done: step.status === 'completed',
    })),
  );
  /**
   * Each step being written: its words, its field's name with its place, and the id of its
   * words, which describe its remove button.
   */
  protected readonly fields = computed(() =>
    (this.draft() ?? []).map((text, index) => ({
      text,
      label: `${this.stepLabel()} ${String(index + 1)}`,
      id: `${this.fieldId}-${String(index)}`,
    })),
  );

  protected startEditing(): void {
    this.draft.set(this.steps().map((step) => step.step));
    this.focusAfter(0);
  }

  protected write(index: number, event: Event): void {
    const value = (event.target as HTMLInputElement).value;
    this.draft.update((draft) => (draft ?? []).map((step, at) => (at === index ? value : step)));
  }

  protected remove(index: number): void {
    const left = (this.draft() ?? []).filter((_, at) => at !== index);
    this.draft.set(left);
    this.focusAfter(left.length === 0 ? 'add' : Math.min(index, left.length - 1));
  }

  protected add(): void {
    const draft = [...(this.draft() ?? []), ''];
    this.draft.set(draft);
    this.focusAfter(draft.length - 1);
  }

  protected save(): void {
    const steps = (this.draft() ?? []).map((step) => step.trim()).filter((step) => step !== '');
    if (steps.length === 0 || this.busy()) {
      return;
    }
    this.edit.emit(steps);
    this.draft.set(null);
    this.focusAfter('summary');
  }

  protected cancel(): void {
    this.draft.set(null);
    this.focusAfter('summary');
  }

  /** Moves the keyboard to `focus` once the card has drawn what changed. */
  private focusAfter(focus: Focus): void {
    afterNextRender(
      () => {
        const target =
          focus === 'summary'
            ? this.summaryText()
            : focus === 'add'
              ? this.addButton()
              : this.stepFields().at(focus);
        const element: unknown = target?.nativeElement;
        if (element instanceof HTMLElement) {
          element.focus();
        }
      },
      { injector: this.injector },
    );
  }
}
