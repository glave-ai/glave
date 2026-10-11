import { InjectionToken, inject } from '@angular/core';

/** Makes ids unique within one app, for linking labels, hints and popups to their controls. */
export const GLZ_ID_SEQUENCE = new InjectionToken<{ next: number }>('GLZ_ID_SEQUENCE', {
  providedIn: 'root',
  factory: () => ({ next: 0 }),
});

/** A new id starting with `prefix`, unique in this app. Call in an injection context. */
export function injectId(prefix: string): string {
  const sequence = inject(GLZ_ID_SEQUENCE);
  const id = `${prefix}-${String(sequence.next)}`;
  sequence.next += 1;
  return id;
}
