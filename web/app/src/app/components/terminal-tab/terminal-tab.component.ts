import {
  ChangeDetectionStrategy,
  Component,
  afterNextRender,
  computed,
  inject,
} from '@angular/core';
import type { TerminalId } from '@glave/client';
import { GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { Plus, X } from 'lucide';

import { TerminalsService } from '../../services/terminals.service';
import { TerminalScreenComponent } from '../terminal-screen/terminal-screen.component';

/** The size a terminal opens at, before its screen has measured its box. */
const OPENS_AT = { cols: 80, rows: 24 } as const;

/**
 * The thread panel's Terminal tab: the person's own shell in the thread's folder, on their own
 * computer, not the agent's. A line naming the folder and saying whose shell it is, a + for
 * another, a button for each when there are several and one closing the shown one, then its
 * dark screen, filling the tab. Its shell keeps running while the tab is away, and shows again
 * with what it printed meanwhile; one whose shell ended says so.
 */
@Component({
  selector: 'app-terminal-tab',
  imports: [GlzIconButtonComponent, GlzIconComponent, TerminalScreenComponent],
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
  /** A button for each terminal, numbered, once there are several; else `null`. */
  protected readonly choices = computed(() => {
    const terminals = this.service.terminals();
    const shown = this.shown()?.id ?? null;
    if (terminals.length < 2) {
      return null;
    }
    return terminals.map((terminal, index) => {
      const number = String(index + 1);
      return {
        id: terminal.id,
        number,
        label: $localize`:@@terminalTab.show:Terminal ${number}:NUMBER:`,
        pressed: terminal.id === shown,
      };
    });
  });
  protected readonly waiting = computed(() => this.service.opening() && this.shown() === null);

  constructor() {
    afterNextRender(() => {
      void this.service.start(OPENS_AT);
    });
  }

  protected another(): void {
    void this.service.open(OPENS_AT);
  }

  protected show(terminal: TerminalId): void {
    this.service.show(terminal);
  }

  protected close(terminal: TerminalId): void {
    void this.service.close(terminal);
  }
}
