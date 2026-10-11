import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import {
  GlzBadgeComponent,
  GlzBreadcrumbComponent,
  GlzBreadcrumbsComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzNavItemComponent,
  GlzNavListComponent,
  GlzRailComponent,
  GlzRailItemComponent,
  GlzSidenavLayoutComponent,
  GlzTabComponent,
  GlzTabsComponent,
  GlzTopBarComponent,
} from '@glave/glaze';
import { Brain, Hash, ListChecks, Menu, MessagesSquare, Settings } from 'lucide';

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
  breadcrumbs: 'Breadcrumbs',
  space: 'glave',
  threads: 'Threads',
  thread: 'Plan the launch',
  nav: 'Rail, side nav and lists',
  main: 'Main',
  chat: 'Chat',
  tasks: 'Tasks',
  memory: 'Memory',
  settings: 'Settings',
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
  layoutTitle: 'glave',
} as const;

/** The showcase's tabs, breadcrumbs, rail, side nav, lists and top bar. */
@Component({
  selector: 'app-showcase-layout',
  imports: [
    GlzBadgeComponent,
    GlzBreadcrumbComponent,
    GlzBreadcrumbsComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzNavItemComponent,
    GlzNavListComponent,
    GlzRailComponent,
    GlzRailItemComponent,
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
    chat: MessagesSquare,
    tasks: ListChecks,
    memory: Brain,
    settings: Settings,
    menu: Menu,
    thread: Hash,
  };
  protected readonly tab = signal(0);
  protected readonly values = { link: '#layout', narrow: '(max-width: 40rem)' } as const;
}
