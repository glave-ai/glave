/**
 * Host entries that keep a control out of reach until Angular first binds it. Angular makes a
 * view's elements with their static attributes, then binds them: a button's name (its `label`,
 * the text in it), its classes, its state. A view it adds outside change detection (a deferred
 * block once its code arrives, a route's page) is in the page unbound until the next change
 * detection, a task later, and a button there has no name until then. So the control starts
 * `inert`, and the change detection that names it takes that away: the keyboard, the pointer
 * and screen readers meet it named, never before. Code that reaches for it in that first change
 * detection (an effect on its first run) finds it out of reach too, so what moves focus waits
 * for the page to be rendered (`afterRenderEffect`, `afterNextRender`), as the dialog does.
 * Glaze's controls spread it into their `host`: its buttons, nav items and menus, its fields
 * (`glzInput`, the composer's box, checkboxes and radios, named by a label bound or projected)
 * and its disclosures (receipts and steps, each a `summary` named by its line). A control a
 * template makes only in a block (`@if`), as the line of the model's reasoning does, is made as
 * it is bound and needs none. A host directive would do the same, for a kilobyte more of the
 * first load.
 */
export const INERT_UNTIL_BOUND = { inert: '', '[attr.inert]': 'null' } as const;
