import { ChangeDetectionStrategy, Component, computed, signal } from '@angular/core';
import {
  GlzBadgeComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzMenuButtonComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
  GlzPanelLayoutComponent,
  GlzSidenavLayoutComponent,
  GlzTabComponent,
  GlzTabsComponent,
  GlzTopBarComponent,
} from '@glave/glaze';
import { Hand, Hash, LoaderCircle, Menu, PanelRight, Plus } from 'lucide';

const COPY = {
  heading: 'Layout and navigation',
  tabs: 'Tabs',
  tabsLabel: 'Thread details',
  conversation: 'Conversation',
  events: 'Events',
  cost: 'Cost',
  conversationText: 'The messages, as the person and the model wrote them.',
  eventsText: 'Every event in the log, in order.',
  costText: 'Tokens and money, call by call.',
  threads: 'Threads',
  nav: 'Side nav, lists and panel',
  menu: 'Show or hide the side nav',
  sideNav: 'Spaces and threads',
  today: 'Today',
  earlier: 'Earlier',
  plan: 'Plan the launch',
  build: 'Fix the build',
  waiting: '1 approval',
  running: 'running',
  notes: 'Release notes',
  content: 'The page goes here. On a narrow screen the side nav slides over it instead.',
  layoutTitle: 'Atlas',
  newIn: 'New thread in Earlier',
  more: 'Options for Earlier',
  needsYou: 'needs you',
  details: 'Details',
  detailsToggle: 'Show or hide the details',
  detailsText: 'A panel at the end: drag its edge, or use the arrow keys on it, to resize it.',
} as const;

/** The showcase's tabs, side nav, lists, panel and top bar. */
@Component({
  selector: 'app-showcase-layout',
  imports: [
    GlzBadgeComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzMenuButtonComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    GlzPanelLayoutComponent,
    GlzSidenavLayoutComponent,
    GlzTabComponent,
    GlzTabsComponent,
    GlzTopBarComponent,
  ],
  templateUrl: './showcase-layout.component.html',
  styleUrl: './showcase-layout.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseLayoutComponent {
  protected readonly copy = COPY;
  protected readonly icons = {
    menu: Menu,
    thread: Hash,
    panel: PanelRight,
    add: Plus,
    running: LoaderCircle,
    waiting: Hand,
  };
  protected readonly tab = signal(0);
  protected readonly earlier = signal(true);
  /** Closed, Earlier shows that something in it waits. */
  protected readonly earlierClosed = computed(() => !this.earlier());
  protected readonly actions = [
    { value: 'rename', label: 'Rename' },
    { value: 'archive', label: 'Archive' },
  ] as const;
  protected readonly details = signal(true);
  protected readonly detailsWidth = signal<number | null>(null);
  protected readonly values = { link: '#layout', narrow: '(max-width: 40rem)' } as const;
}
