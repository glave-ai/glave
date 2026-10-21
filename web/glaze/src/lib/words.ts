import { inject } from '@angular/core';

import { GLZ_TEXT, type GlzText } from '../tokens/text';

/**
 * The words of `part`: the app's (`GLZ_TEXT`) when it gives them, else `english`, which each
 * component keeps beside itself. Call in an injection context.
 */
export function glzText<K extends keyof GlzText>(part: K, english: GlzText[K]): GlzText[K] {
  return inject(GLZ_TEXT, { optional: true })?.[part] ?? english;
}
