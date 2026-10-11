import { InjectionToken, type Provider, inject } from '@angular/core';

import type { GlzIconData } from '../lib/icons';

/** Icons registered by name, for `<glz-icon name="…">`. */
export const GLZ_ICONS = new InjectionToken<ReadonlyMap<string, GlzIconData>>('GLZ_ICONS', {
  providedIn: 'root',
  factory: () => new Map(),
});

/**
 * Registers icons by name, adding to any registered above. Only the icons imported are
 * bundled: Lucide ships each icon as its own ES module.
 *
 * ```ts
 * import { Send, Square } from 'lucide';
 * providers: [provideGlazeIcons({ send: Send, stop: Square })]
 * ```
 */
export function provideGlazeIcons(icons: Readonly<Record<string, GlzIconData>>): Provider {
  return {
    provide: GLZ_ICONS,
    useFactory: (): ReadonlyMap<string, GlzIconData> => {
      const parent = inject(GLZ_ICONS, { skipSelf: true, optional: true });
      return new Map([...(parent ?? new Map<string, GlzIconData>()), ...Object.entries(icons)]);
    },
  };
}
