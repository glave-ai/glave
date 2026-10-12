import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import {
  GlzAvatarComponent,
  GlzBadgeComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzCopyDirective,
  GlzEmptyStateComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTooltipDirective,
} from '@glave/glaze';
import { Copy, MessagesSquare, Pencil, Plus, Trash2 } from 'lucide';

const COPY = {
  heading: 'Actions and display',
  buttons: 'Buttons',
  primary: 'Save',
  secondary: 'Cancel',
  ghost: 'More',
  danger: 'Delete',
  small: 'Small',
  large: 'Large',
  link: 'A link that looks like a button',
  iconButtons: 'Icon buttons and tooltips',
  add: 'Add',
  edit: 'Edit',
  remove: 'Delete',
  editHint: 'Rename this thread',
  copyLabel: 'Copy',
  copyText: 'cargo xtask ci',
  copied: 'Copied',
  badges: 'Badges and avatars',
  neutral: 'idle',
  accent: 'new',
  success: 'done',
  warning: '2 approvals',
  dangerBadge: 'failed',
  person: 'Ada Lovelace',
  hope: 'Hope',
  cards: 'Cards and empty states',
  card: 'On the card surface',
  raised: 'Raised',
  sunken: 'Sunken',
  cardBody: '1.2M tokens this week.',
  emptyHeading: 'No threads yet',
  emptyText: 'Start one and it shows up here.',
  newChat: 'New chat',
} as const;

/** The demo's values that are not words. */
const VALUES = { link: '#actions' } as const;

/** The showcase's buttons, badges, avatars, cards and empty states. */
@Component({
  selector: 'app-showcase-actions',
  imports: [
    GlzAvatarComponent,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzCopyDirective,
    GlzEmptyStateComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzTooltipDirective,
  ],
  templateUrl: './showcase-actions.component.html',
  styleUrl: './showcase-actions.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseActionsComponent {
  protected readonly copy = COPY;
  protected readonly icons = {
    add: Plus,
    edit: Pencil,
    remove: Trash2,
    copy: Copy,
    chat: MessagesSquare,
  };
  protected readonly values = VALUES;
  protected readonly copyLabel = signal<string>(COPY.copyLabel);

  protected copied(ok: boolean): void {
    if (ok) {
      this.copyLabel.set(COPY.copied);
    }
  }
}
