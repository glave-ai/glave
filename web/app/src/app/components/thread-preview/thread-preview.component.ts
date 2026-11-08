import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  afterRenderEffect,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import type { Preview } from '@glave/client';
import {
  GlzButtonComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { ExternalLink, RotateCw } from 'lucide';

import { frameable, previewKey } from '../../lib/previews';
import { CommandsService } from '../../services/commands.service';
import { ThreadPanelService } from '../../services/thread-panel.service';

/** The frame the tab draws, drawn afresh each time its key changes. */
interface Frame {
  readonly key: string;
  readonly url: string;
  readonly title: string;
}

/**
 * The panel's Preview tab: what one of the thread's commands in the background serves, beside
 * the thread. A bar with the preview chosen, where there are several, its address, Reload and
 * Open in a new tab; under it the page itself, in a frame at its own origin on this computer,
 * never glave's (`specs/session.md`, "Previews"), sandboxed: its scripts, forms and popups run,
 * at its own origin, so a dev server's modules and live reload work; it cannot move this page
 * or open dialogs over it. A page that refuses to be framed stays blank, which the note under
 * it says, and opens in a tab of its own. Only what the server gives as a preview, and only a
 * page on this computer's loopback (`frameable`), is framed: the frame's address is set on it
 * as it is drawn, since a template's would need Angular's bypass, a Trusted Types policy
 * glave's Content Security Policy refuses.
 */
@Component({
  selector: 'app-thread-preview',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent, GlzMenuSelectComponent],
  templateUrl: './thread-preview.component.html',
  styleUrl: './thread-preview.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadPreviewComponent {
  protected readonly copy = {
    which: $localize`:@@threadPreview.which:Preview`,
    reload: $localize`:@@threadPreview.reload:Reload the preview`,
    open: $localize`:@@threadPreview.open:Open in a new tab`,
    note: $localize`:@@threadPreview.note:Some apps refuse to be shown inside another page. If this stays blank, open it in a new tab.`,
  } as const;
  protected readonly icons = { reload: RotateCw, open: ExternalLink } as const;
  private readonly facts = inject(ThreadPanelService);
  private readonly commands = inject(CommandsService);
  private readonly frame = viewChild<ElementRef<HTMLIFrameElement>>('frame');
  /** Counts the reloads asked for: each draws the frame afresh. */
  private readonly loads = signal(0);

  /** The preview shown. */
  protected readonly preview = this.facts.preview;
  /** The previews to choose among, when there are several. */
  protected readonly options = computed((): readonly GlzMenuOption[] => {
    const previews = this.facts.previews();
    return previews.length < 2
      ? []
      : previews.map((preview) => ({
          value: previewKey(preview),
          label: $localize`:@@threadPreview.option:${this.serving(preview)}:COMMAND:, port ${String(preview.port)}:PORT:`,
        }));
  });
  protected readonly choosing = computed(() => this.options().length > 0);
  protected readonly chosen = computed(() => {
    const preview = this.preview();
    return preview === null ? null : previewKey(preview);
  });
  /** The frame of the preview shown, when its address is one the app frames. */
  protected readonly frames = computed((): readonly Frame[] => {
    const preview = this.preview();
    const url = preview === null ? null : frameable(preview);
    if (preview === null || url === null) {
      return [];
    }
    const what = this.serving(preview);
    return [
      {
        key: `${previewKey(preview)}#${String(this.loads())}`,
        url,
        title: $localize`:@@threadPreview.title:What ${what}:COMMAND: serves`,
      },
    ];
  });

  constructor() {
    // Each frame drawn is sent to its page, once.
    afterRenderEffect(() => {
      const frame = this.frame()?.nativeElement;
      const url = this.frames()[0]?.url;
      if (frame !== undefined && url !== undefined && frame.getAttribute('src') !== url) {
        frame.src = url;
      }
    });
  }

  protected choose(key: string | null): void {
    if (key !== null) {
      this.facts.choosePreview(key);
    }
  }

  protected reload(): void {
    this.loads.update((loads) => loads + 1);
  }

  /** What the command serving `preview` runs, as the agent wrote it. */
  private serving(preview: Preview): string {
    return this.commands.commands().find((each) => each.id === preview.command)?.command ?? '';
  }
}
