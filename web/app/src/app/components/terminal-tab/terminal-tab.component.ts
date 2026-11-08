import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  afterNextRender,
  computed,
  inject,
} from '@angular/core';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  type GlzOption,
  GlzSegmentedComponent,
} from '@glave/glaze';
import { Plus, X } from 'lucide';

import { TerminalsService } from '../../services/terminals.service';
import { TerminalScreenComponent } from '../terminal-screen/terminal-screen.component';

/** The size a terminal opens at, before its screen has measured its box. */
const OPENS_AT = { cols: 80, rows: 24 } as const;

/**
 * The thread panel's Terminal tab: the person's own shell in the thread's folder, on their own
 * computer, not the agent's. A line naming the folder and saying whose shell it is, a choice
 * among them by number when there are several, a + for another and a button closing the shown
 * one, then its dark screen, filling the tab. Its shell keeps running while the tab is away, and
 * shows again with what it printed meanwhile; one whose shell ended says so. While it shows, the
 * thread's terminals are listed again every so often.
 */
@Component({
  selector: 'app-terminal-tab',
  imports: [
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzSegmentedComponent,
    TerminalScreenComponent,
  ],
  providers: [TerminalsService],
  templateUrl: './terminal-tab.component.html',
  styleUrl: './terminal-tab.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TerminalTabComponent {
  protected readonly copy = {
    who: $localize`:@@terminalTab.who:Your shell, not the agent's`,
    another: $localize`:@@terminalTab.another:Open another terminal`,
    close: $localize`:@@terminalTab.close:Close this terminal`,
    ended: $localize`:@@terminalTab.ended:This shell has ended. Close it, or open another.`,
    opening: $localize`:@@terminalTab.opening:Opening your terminal…`,
    terminals: $localize`:@@terminalTab.terminals:Terminals`,
  } as const;
  protected readonly icons = { another: Plus, close: X } as const;
  protected readonly service = inject(TerminalsService);
  protected readonly shown = this.service.shown;
  /** The shown terminal alone, so its screen is made anew when another shows. */
  protected readonly screens = computed(() => {
    const shown = this.shown();
    return shown === null ? [] : [shown];
  });
  /** Each terminal as a choice, by its number, once there are several; else `null`. */
  protected readonly choices = computed((): readonly GlzOption[] | null => {
    const terminals = this.service.terminals();
    if (terminals.length < 2) {
      return null;
    }
    return terminals.map((terminal, index) => ({ value: terminal.id, label: String(index + 1) }));
  });
  /** The id of the one shown, which the choice marks. */
  protected readonly chosen = computed(() => this.shown()?.id ?? null);
  protected readonly waiting = computed(() => this.service.opening() && this.shown() === null);

  constructor() {
    const stop = new AbortController();
    inject(DestroyRef).onDestroy(() => {
      stop.abort();
    });
    afterNextRender(() => {
      void this.service.start(OPENS_AT);
      void this.service.watch(stop.signal);
    });
  }

  protected another(): void {
    void this.service.open(OPENS_AT);
  }

  protected show(terminal: string | null): void {
    if (terminal !== null) {
      this.service.show(terminal);
    }
  }

  protected close(terminal: string): void {
    void this.service.close(terminal);
  }
}
