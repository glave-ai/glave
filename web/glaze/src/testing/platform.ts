// Unit tests run in jsdom, which lacks the modal dialog and popover APIs Glaze builds on. These
// stand-ins do what the tests observe (the `open` state, `close` and `toggle` events); the real
// behaviour (top layer, focus, light dismiss) is checked in a browser, end to end.

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
}
