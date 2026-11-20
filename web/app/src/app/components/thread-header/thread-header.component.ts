import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  booleanAttribute,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  viewChild,
} from '@angular/core';
import type { ScheduleInfo } from '@glave/client';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  type GlzOption,
  GlzTooltipDirective,
} from '@glave/glaze';
import { Folder, Menu, PanelRight, Repeat } from 'lucide';

import { repeatLine } from '../../lib/repeating';
import { clock } from '../../services/clock';
import { NavService } from '../../services/nav.service';
import { PanelService } from '../../services/panel.service';
import { READER_ZONE } from '../../services/zone.service';
import { ScheduleActionsComponent } from '../schedule-actions/schedule-actions.component';

/** What the ⋯ on a phone offers for the panel, in its button's place. */
const PANEL = 'panel';

/**
 * A thread's bar across its top, ruled off from the conversation: its title, which renames it
 * when pressed; a quiet line with its project and, for a schedule's own chat, when it repeats
 * and runs next on the schedule's clocks, with Pause or Resume and Remove, and what the server
 * refused them with under the bar; and at the end, when the thread has a panel of facts, the
 * button that shows or hides it: a toggle, pressed while it shows, beside a wide page; on a
 * narrower one, where it opens a sheet over the page, a button that says whether the sheet is
 * expanded. On a phone the title and that line sit in the middle, between the button that opens
 * the side bar and the panel's, each a finger's size; a schedule's chat says there only when it
 * repeats, and its actions, the panel among them, sit under a ⋯ in the panel button's place, so
 * the bar keeps to one row.
 */
@Component({
  selector: 'app-thread-header',
  imports: [
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzTooltipDirective,
    ScheduleActionsComponent,
  ],
  templateUrl: './thread-header.component.html',
  styleUrl: './thread-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadHeaderComponent {
  /** The thread's title. */
  readonly title = input.required<string>();
  /** The name of the project it is in; `null` for a chat. */
  readonly project = input<string | null>(null);
  /** For a schedule's own chat, its schedule, as the thread's summary says; else `null`. */
  readonly schedule = input<ScheduleInfo | null>(null);
  /** Whether the thread has a panel of facts to show or hide. */
  readonly panel = input(false, { transform: booleanAttribute });
  /** Emitted when the person asks to rename the thread. */
  readonly rename = output();

  protected readonly copy = {
    menu: $localize`:@@threadHeader.menu:Show or hide the side bar`,
    rename: $localize`:@@threadHeader.rename:Rename the chat`,
    panel: $localize`:@@threadHeader.panel:Show or hide the chat's panel`,
  } as const;
  protected readonly icons = { menu: Menu, folder: Folder, panel: PanelRight, repeat: Repeat };
  private readonly nav = inject(NavService);
  private readonly panels = inject(PanelService);
  private readonly locale = inject(LOCALE_ID);
  private readonly zone = inject(READER_ZONE);
  private readonly injector = inject(Injector);
  private readonly heading = viewChild.required<ElementRef<HTMLElement>>('heading');
  /** A phone: the side bar is a drawer, opened from this bar. */
  protected readonly phone = this.nav.narrow;
  protected readonly navOpen = this.nav.open;
  protected readonly navShortcut = this.nav.ariaShortcut;
  protected readonly panelShortcut = this.panels.ariaShortcut;
  /** Beside a wide page: whether the toggle is pressed. */
  protected readonly pressed = computed(() =>
    this.panels.narrow() ? null : String(this.panels.showing()),
  );
  /** Over a narrow page: whether the sheet it opens is expanded. */
  protected readonly expanded = computed(() =>
    this.panels.narrow() ? String(this.panels.showing()) : null,
  );
  /**
   * The schedule as the bar shows it: the summary's, until a change made here is answered,
   * which shows at once, and again the summary's as the server sends it anew.
   */
  protected readonly shown = linkedSignal(() => this.schedule());
  /** What the server refused a change to the schedule with, in its words, until another chat's. */
  protected readonly error = linkedSignal<string | null, string | null>({
    source: () => this.schedule()?.schedule ?? null,
    computation: () => null,
  });
  /** Now, moving on while a schedule shows, so "tomorrow" becomes "today". */
  private readonly now = clock(() => this.shown() !== null);
  /**
   * When the schedule repeats and runs next, in a line; on a phone, when it repeats alone, in the
   * server's words; `null` for any other thread.
   */
  protected readonly line = computed(() => {
    const schedule = this.shown();
    const reader = { locale: this.locale, timeZone: this.zone };
    if (schedule === null) {
      return null;
    }
    return this.phone() ? schedule.repeats : repeatLine(schedule, reader, this.now());
  });
  protected readonly meta = computed(() => this.project() !== null || this.shown() !== null);
  /** The schedule its actions are buttons for, in the line beside a wide page. */
  protected readonly buttons = computed(() => (this.phone() ? null : this.shown()));
  /** The schedule its actions are a ⋯ for, in the panel button's place on a phone. */
  protected readonly menu = computed(() => (this.phone() ? this.shown() : null));
  /** Whether the panel's button shows: beside a wide page, or on a phone with no ⋯ in its place. */
  protected readonly panelButton = computed(() => this.panel() && this.menu() === null);
  /** What the ⋯ also offers on a phone: the panel, in its button's place. */
  protected readonly extras = computed<readonly GlzOption[]>(() =>
    this.panel() ? [{ value: PANEL, label: this.copy.panel }] : [],
  );

  protected startRename(): void {
    this.rename.emit();
  }

  protected toggleNav(): void {
    this.nav.toggle();
  }

  protected togglePanel(): void {
    this.panels.toggle();
  }

  /** One of the ⋯'s extras was picked on a phone: the panel's. */
  protected extra(value: string): void {
    if (value === PANEL) {
      this.togglePanel();
    }
  }

  protected changed(schedule: ScheduleInfo): void {
    this.shown.set(schedule);
  }

  /** The schedule went, and its line with it: the keyboard goes to the title. */
  protected removed(): void {
    this.shown.set(null);
    afterNextRender(() => this.heading().nativeElement.focus(), { injector: this.injector });
  }
}
