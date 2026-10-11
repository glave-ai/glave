import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  input,
} from '@angular/core';
import { X } from 'lucide';

import { GlzToastService } from '../../services/toast.service';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';

/**
 * Where `GlzToastService`'s toasts show: place it once, at the end of the app's root
 * template. A native popover, so toasts sit in the top layer, over everything but a modal
 * dialog. Screen readers hear each toast from the service as it comes; this region holds
 * their actions.
 *
 * ```html
 * <glz-toast-stack />
 * ```
 */
@Component({
  selector: 'glz-toast-stack',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './toast-stack.component.html',
  styleUrl: './toast-stack.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'region',
    popover: 'manual',
    '[attr.aria-label]': 'label()',
  },
})
export class GlzToastStackComponent {
  readonly label = input('Notifications');
  readonly dismissLabel = input('Dismiss');

  private readonly service = inject(GlzToastService);
  protected readonly closeIcon = X;
  protected readonly shown = computed(() =>
    this.service.toasts().map((toast) => ({ ...toast, toneClass: `toast tone-${toast.tone}` })),
  );

  protected dismiss(id: number): void {
    this.service.dismiss(id);
  }

  constructor() {
    const host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    effect(() => {
      if (this.shown().length > 0) {
        host.showPopover();
      } else {
        host.hidePopover();
      }
    });
  }

  protected act(id: number): void {
    this.service.act(id);
  }
}
