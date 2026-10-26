import type { Permissions } from '@glave/client';
import type { GlzIconData } from '@glave/glaze';
import { ClipboardList, Hand, ShieldCheck, ShieldOff } from 'lucide';

/** How a mode is shown: its word, the line saying what it means, and its icon. */
export interface Mode {
  readonly text: string;
  readonly about: string;
  readonly icon: GlzIconData;
  /** Whether a picker names it as the one glave recommends. */
  readonly recommended: boolean;
}

/** How each mode is shown, in the words of the language the app reads in. */
export function modes(): Readonly<Record<Permissions, Mode>> {
  return {
    auto: {
      text: $localize`:@@permissions.auto:Auto`,
      about: $localize`:@@permissions.autoAbout:Works on its own in this project. Asks only before something risky.`,
      icon: ShieldCheck,
      recommended: true,
    },
    plan: {
      text: $localize`:@@permissions.plan:Plan`,
      about: $localize`:@@permissions.planAbout:Looks around and proposes a plan. Changes nothing until you approve it.`,
      icon: ClipboardList,
      recommended: false,
    },
    ask: {
      text: $localize`:@@permissions.ask:Ask first`,
      about: $localize`:@@permissions.askAbout:Asks you before every change.`,
      icon: Hand,
      recommended: false,
    },
    'full-access': {
      text: $localize`:@@permissions.fullAccess:Full access`,
      about: $localize`:@@permissions.fullAccessAbout:No limits: can change anything on this computer, and never asks.`,
      icon: ShieldOff,
      recommended: false,
    },
  };
}
