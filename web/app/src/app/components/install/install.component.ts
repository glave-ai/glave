import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  inject,
  viewChild,
} from '@angular/core';
import { GlzButtonComponent, GlzIconComponent, injectId, positionPopover } from '@glave/glaze';
import { MonitorDown } from 'lucide';

import type { InstallOffer } from '../../lib/install';
import { InstallService } from '../../services/install.service';

const COPY = {
  install: 'Install app',
  heading: 'Install glave',
} as const;

/** The one step Safari needs, where it has no prompt of its own. */
const STEPS: Readonly<Record<InstallOffer, string | null>> = {
  prompt: null,
  none: null,
  'safari-mac': 'In Safari’s menu bar, choose File, then Add to Dock.',
  'safari-ios': 'Tap Share in Safari’s toolbar, then Add to Home Screen.',
};

/**
 * "Install app", at the foot of the side nav: the browser's own install prompt where it has
 * one (Chromium), a popover with Safari's one step where it does not, and nothing once glave
 * runs as an installed app or where the page knows no way.
 */
@Component({
  selector: 'app-install',
  imports: [GlzButtonComponent, GlzIconComponent],
  templateUrl: './install.component.html',
  styleUrl: './install.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class InstallComponent {
  protected readonly copy = COPY;
  protected readonly icon = MonitorDown;
  protected readonly popoverId = injectId('app-install');
  private readonly service = inject(InstallService);
  private readonly trigger = viewChild<string, ElementRef<HTMLElement>>('trigger', {
    read: ElementRef,
  });
  private readonly how = viewChild<ElementRef<HTMLElement>>('how');
  protected readonly prompting = computed(() => this.service.offer() === 'prompt');
  protected readonly steps = computed(() => STEPS[this.service.offer()]);

  protected install(): void {
    void this.service.install();
  }

  /** Places the steps above the button as they open. */
  protected toggled(event: Event): void {
    const trigger = this.trigger()?.nativeElement;
    const how = this.how()?.nativeElement;
    const opened = 'newState' in event && event.newState === 'open';
    if (opened && trigger !== undefined && how !== undefined) {
      positionPopover(trigger, how, 'above', `--${this.popoverId}`);
    }
  }
}
