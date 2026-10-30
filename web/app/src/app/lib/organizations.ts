import type { OrganizationInfo } from '@glave/client';

import { plural } from './plural';

/** An organization as the person's menu lists it: what it is called, and who is in it. */
export interface Place {
  readonly id: string;
  readonly name: string;
  readonly note: string;
  readonly current: boolean;
  /** The first letter of its name, for its mark. */
  readonly letter: string;
}

/** `organization` as the person's menu lists it, in `locale`, by the name the server gives it. */
export function place(organization: OrganizationInfo, locale: string): Place {
  const note =
    organization.own === true && organization.people <= 1
      ? $localize`:@@organizations.justYou:Just you`
      : plural(
          $localize`:@@organizations.people:{${organization.people}:COUNT:, plural, one {# person} other {# people}}`,
          locale,
        );
  return {
    id: organization.id,
    name: organization.name,
    note,
    current: organization.current === true,
    letter: organization.name.charAt(0).toUpperCase(),
  };
}
