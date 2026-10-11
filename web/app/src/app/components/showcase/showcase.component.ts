import { ChangeDetectionStrategy, Component, computed, signal } from '@angular/core';
import { GlzNavItemComponent, GlzNavListComponent, GlzScrollSpyDirective } from '@glave/glaze';

import { ShowcaseActionsComponent } from '../showcase-actions/showcase-actions.component';
import { ShowcaseAiComponent } from '../showcase-ai/showcase-ai.component';
import { ShowcaseFormsComponent } from '../showcase-forms/showcase-forms.component';
import { ShowcaseLayoutComponent } from '../showcase-layout/showcase-layout.component';
import { ShowcaseOverlaysComponent } from '../showcase-overlays/showcase-overlays.component';

/** The showcase's sections, in order. */
const SECTIONS = [
  { id: 'actions', title: 'Actions and display' },
  { id: 'forms', title: 'Forms' },
  { id: 'layout', title: 'Layout and navigation' },
  { id: 'overlays', title: 'Dialogs, menus and toasts' },
  { id: 'ai', title: 'Conversation' },
] as const;

const COPY = {
  heading: 'Glaze',
  intro:
    'Every Glaze component and directive, live. The look is placeholder: everything comes from design tokens, so a new design is a new set of CSS custom properties.',
  contents: 'On this page',
} as const;

/** `/glaze`: every Glaze component on one page, to see and try. */
@Component({
  selector: 'app-showcase',
  imports: [
    GlzNavListComponent,
    GlzNavItemComponent,
    GlzScrollSpyDirective,
    ShowcaseActionsComponent,
    ShowcaseFormsComponent,
    ShowcaseLayoutComponent,
    ShowcaseOverlaysComponent,
    ShowcaseAiComponent,
  ],
  templateUrl: './showcase.component.html',
  styleUrl: './showcase.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseComponent {
  protected readonly copy = COPY;
  protected readonly active = signal<string | null>(null);
  protected readonly sections = computed(() =>
    SECTIONS.map((section) => ({
      ...section,
      href: `#${section.id}`,
      current: section.id === this.active(),
    })),
  );
}
