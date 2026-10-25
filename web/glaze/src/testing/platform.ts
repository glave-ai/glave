// Unit tests run in jsdom, which lacks the modal dialog and popover APIs Glaze builds on. These
// stand-ins do what the tests observe (the `open` state, `close` and `toggle` events, and,
// roughly, a press outside a popover closing it); the real behaviour (top layer, focus, light
// dismiss as browsers do it) is checked in a browser, end to end.

const popovers = new WeakSet<HTMLElement>();

/** Whether `element` is an open popover, as `:popover-open` would say. */
export function popoverOpen(element: Element | null): boolean {
  return element instanceof HTMLElement && popovers.has(element);
}

if (typeof HTMLDialogElement.prototype.showModal !== 'function') {
  Object.assign(HTMLDialogElement.prototype, {
    showModal(this: HTMLDialogElement): void {
      this.setAttribute('open', '');
      this.dataset['modal'] = 'true';
    },
    show(this: HTMLDialogElement): void {
      this.setAttribute('open', '');
    },
    close(this: HTMLDialogElement, returnValue?: string): void {
      if (!this.hasAttribute('open')) {
        return;
      }
      this.removeAttribute('open');
      delete this.dataset['modal'];
      if (returnValue !== undefined) {
        this.returnValue = returnValue;
      }
      this.dispatchEvent(new Event('close'));
    },
  });
}

if (typeof HTMLElement.prototype.showPopover !== 'function') {
  const toggled = (element: HTMLElement, open: boolean): void => {
    const event = new Event('toggle');
    Object.assign(event, {
      newState: open ? 'open' : 'closed',
      oldState: open ? 'closed' : 'open',
    });
    element.dispatchEvent(event);
  };
  Object.assign(HTMLElement.prototype, {
    showPopover(this: HTMLElement): void {
      if (!popovers.has(this)) {
        popovers.add(this);
        toggled(this, true);
      }
    },
    hidePopover(this: HTMLElement): void {
      if (popovers.has(this)) {
        popovers.delete(this);
        toggled(this, false);
      }
    },
    togglePopover(this: HTMLElement): boolean {
      if (popovers.has(this)) {
        this.hidePopover();
        return false;
      }
      this.showPopover();
      return true;
    },
  });
  // Light dismiss: a press outside an open popover that is not `manual` hides it, unless the
  // press is on a button that opens it (`popovertarget`, `popoverTargetElement`), which the
  // platform counts as inside.
  const opens = (button: HTMLButtonElement | null, popover: HTMLElement): boolean =>
    button !== null &&
    (button.popoverTargetElement === popover ||
      (popover.id !== '' && button.getAttribute('popovertarget') === popover.id));
  document.addEventListener(
    'pointerdown',
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const button = target?.closest('button') ?? null;
      for (const popover of Array.from(document.querySelectorAll<HTMLElement>('[popover]'))) {
        const outside = target === null || !popover.contains(target);
        const dismissed = popover.getAttribute('popover') !== 'manual' && popovers.has(popover);
        if (dismissed && outside && !opens(button, popover)) {
          popover.hidePopover();
        }
      }
    },
    true,
  );
}

// jsdom runs no animations: nothing is ever moving.
if (typeof Element.prototype.getAnimations !== 'function') {
  Object.assign(Element.prototype, {
    getAnimations(): Animation[] {
      return [];
    },
  });
}

// jsdom does no layout, so no size ever changes: what watches sizes hears nothing.
if (typeof globalThis.ResizeObserver !== 'function') {
  Object.assign(globalThis, {
    ResizeObserver: class {
      observe(): void {
        // Nothing to watch without layout.
      }

      unobserve(): void {
        // Nor to stop watching.
      }

      disconnect(): void {
        // Nor to let go of.
      }
    },
  });
}

// jsdom does no layout, so it scrolls nothing into view.
if (typeof Element.prototype.scrollIntoView !== 'function') {
  Object.assign(Element.prototype, {
    scrollIntoView(): void {
      // Nothing to scroll without layout.
    },
  });
}
