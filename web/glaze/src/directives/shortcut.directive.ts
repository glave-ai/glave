import {
  DestroyRef,
  Directive,
  ElementRef,
  booleanAttribute,
  computed,
  inject,
  input,
  output,
} from '@angular/core';

import { isApple, isTyping, matchesShortcut, parseShortcut } from '../lib/shortcuts';

/**
 * A keyboard shortcut for its element, anywhere on the page: `mod` is Cmd on Apple platforms
 * and Ctrl elsewhere. It clicks the element (so a button's own action runs) and emits
 * `pressed`. Shortcuts without `mod` or `alt` wait while the person types in a field, unless
 * `whileTyping` is set. Show the shortcut in the element's label or tooltip too.
 *
 * ```html
 * <button glzButton glzShortcut="mod+shift+o" (click)="newChat()">New chat</button>
 * ```
 */
@Directive({ selector: '[glzShortcut]' })
export class GlzShortcutDirective {
  readonly keys = input.required<string>({ alias: 'glzShortcut' });
  /** Whether the shortcut works while typing in a field. */
  readonly whileTyping = input(false, { transform: booleanAttribute });
  readonly pressed = output();

  private readonly element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private readonly shortcut = computed(() => parseShortcut(this.keys()));

  constructor() {
    const document = this.element.ownerDocument;
    const apple = isApple(document.defaultView?.navigator ?? ({} as Navigator));
    const listener = (event: KeyboardEvent): void => {
      const shortcut = this.shortcut();
      if (!matchesShortcut(event, shortcut, apple)) {
        return;
      }
      const plain = !shortcut.mod && !shortcut.alt;
      if (plain && !this.whileTyping() && isTyping(event.target)) {
        return;
      }
      event.preventDefault();
      this.element.click();
      this.pressed.emit();
    };
    document.addEventListener('keydown', listener);
    inject(DestroyRef).onDestroy(() => {
      document.removeEventListener('keydown', listener);
    });
  }
}
