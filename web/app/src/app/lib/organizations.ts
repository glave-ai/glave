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

/** What a team, not the person's own, is called: by an Owner of it, while teams have no names. */
export function teamName(organization: OrganizationInfo): string {
  const owner = organization.owner ?? null;
  return owner === null
    ? $localize`:@@organizations.team:A team`
    : $localize`:@@organizations.ownersTeam:${owner}:OWNER:'s team`;
}

/** `organization` as the person's menu lists it, in `locale`. */
export function place(organization: OrganizationInfo, locale: string): Place {
  const own = organization.own === true;
  const name = own ? $localize`:@@organizations.personal:Personal` : teamName(organization);
  const note =
    own && organization.people <= 1
      ? $localize`:@@organizations.justYou:Just you`
      : plural(
          $localize`:@@organizations.people:{${organization.people}:COUNT:, plural, one {# person} other {# people}}`,
          locale,
        );
  return {
    id: organization.id,
    name,
    note,
    current: organization.current === true,
    letter: name.charAt(0).toUpperCase(),
  };
}
