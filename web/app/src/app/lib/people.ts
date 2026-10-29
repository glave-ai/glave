import type { InviteInfo, OrgRole, PersonInfo } from '@glave/client';

import { plural } from './plural';

/** A person as the People page shows them. */
export interface PersonRow {
  readonly id: string;
  /** What names them: their name, else their email, else the app's word for who they are. */
  readonly name: string;
  /** Their email under their name, when their name is shown. */
  readonly email: string | null;
  /** Their role, as the app says it, and "you" when they are the reader. */
  readonly role: string;
  /** The roles the reader may give them, theirs among them; none when they can't. */
  readonly roles: readonly { readonly value: OrgRole; readonly label: string }[];
  readonly current: OrgRole;
  /** Whether the reader may change their role. */
  readonly changeable: boolean;
  readonly removable: boolean;
  /** The label of their role's menu, naming them. */
  readonly roleLabel: string;
}

/** An invite waiting, as the People page shows it. */
export interface InviteRow {
  readonly id: string;
  /** Whom it was emailed to, else that it is a link alone. */
  readonly to: string;
  /** Its role, who made it and when it ends. */
  readonly detail: string;
}

/** A role, as the app names it. */
export function roleName(role: OrgRole): string {
  switch (role) {
    case 'owner':
      return $localize`:@@people.owner:Owner`;
    case 'admin':
      return $localize`:@@people.admin:Admin`;
    case 'member':
      return $localize`:@@people.member:Member`;
  }
}

/** What names `person`: their name, else their email, else who they are to the reader. */
function nameOf(person: PersonInfo): string {
  const named = person.name ?? person.email ?? null;
  if (named !== null) {
    return named;
  }
  if (person.you === true) {
    return $localize`:@@people.you:You`;
  }
  return person.local === true
    ? $localize`:@@people.local:The person who runs this glave`
    : $localize`:@@people.someone:Someone without a name or email`;
}

/** `person` as the People page shows them. */
export function personRow(person: PersonInfo): PersonRow {
  const name = nameOf(person);
  const role = roleName(person.role);
  return {
    id: person.id,
    name,
    email: (person.name ?? null) === null ? null : (person.email ?? null),
    role: person.you === true ? $localize`:@@people.roleYou:${role}:ROLE: · you` : role,
    roles: (person.roles ?? []).map((value) => ({ value, label: roleName(value) })),
    current: person.role,
    changeable: (person.roles ?? []).length > 0,
    removable: person.removable === true,
    roleLabel: $localize`:@@people.roleFor:Role for ${name}:NAME:`,
  };
}

/** How long until an invite ends, as the reader reads it ("in 6 days", "tomorrow"). */
export type Until = (at: string) => string;

/** An `Until` in `locale`, counted from `now` (milliseconds since the epoch). */
export function untilIn(locale: string, now: number): Until {
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const HOUR = 3_600_000;
  return (at) => {
    const hours = Math.max(0, Math.round((Date.parse(at) - now) / HOUR));
    return hours < 24
      ? relative.format(hours, 'hour')
      : relative.format(Math.round(hours / 24), 'day');
  };
}

/**
 * `invite` as the People page shows it: whom it waits for, then its role, who made it among
 * `people` (the reader, or by name), and when it ends.
 */
export function inviteRow(
  invite: InviteInfo,
  people: readonly PersonInfo[],
  until: Until,
): InviteRow {
  const role = roleName(invite.role);
  const ends = until(invite.expires);
  const maker = people.find((person) => person.id === invite.by.id);
  const detail =
    maker === undefined
      ? $localize`:@@people.inviteDetail:${role}:ROLE: · ends ${ends}:ENDS:`
      : maker.you === true
        ? $localize`:@@people.inviteByYou:${role}:ROLE: · invited by you · ends ${ends}:ENDS:`
        : $localize`:@@people.inviteBy:${role}:ROLE: · invited by ${nameOf(maker)}:BY: · ends ${ends}:ENDS:`;
  return {
    id: invite.id,
    to: invite.email ?? $localize`:@@people.linkOnly:A link, no email`,
    detail,
  };
}

/** How many people and invites the page says there are ("4 people · 2 invites waiting"). */
export function counted(people: number, invites: number, locale: string): string {
  const many = plural(
    $localize`:@@people.count:{${people}:COUNT:, plural, one {# person} other {# people}}`,
    locale,
  );
  if (invites === 0) {
    return many;
  }
  const waiting = plural(
    $localize`:@@people.invitesCount:{${invites}:COUNT:, plural, one {# invite waiting} other {# invites waiting}}`,
    locale,
  );
  return $localize`:@@people.countWithInvites:${many}:PEOPLE: · ${waiting}:INVITES:`;
}
