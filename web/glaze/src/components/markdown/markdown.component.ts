import {
  ApplicationRef,
  ChangeDetectionStrategy,
  Component,
  type ComponentRef,
  DestroyRef,
  ElementRef,
  EnvironmentInjector,
  ViewEncapsulation,
  afterRenderEffect,
  booleanAttribute,
  computed,
  createComponent,
  effect,
  inject,
  input,
  linkedSignal,
  signal,
} from '@angular/core';

import { loadHighlighter } from '../../lib/highlight';
import { type GlzMarkdown, renderMarkdown } from '../../lib/markdown';
import { GLZ_MARKDOWN_LABELS, type GlzHighlight } from '../../lib/markdown-html';
import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';
import { GlzCopyButtonComponent } from '../copy-button/copy-button.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { copy: 'copy' } as const;

/**
 * Markdown, rendered on the element it is put on: GitHub-flavoured (tables, task lists,
 * strikethrough), safe whatever the text says (`GlzMarkdownRenderer`: raw HTML shows as text,
 * images as links, never fetched; links open in a new tab, marked so), and sanitized again by
 * Angular on the way in, through its Trusted Types policy. Each top-level block is its own
 * element, rendered once: while `streaming`, only the growing last block is redone, and
 * half-written markdown shows as it will when finished. Code blocks get a copy button, and
 * highlighting loaded the first time their language appears. It flows as prose (`glz-flow`):
 * its blocks spaced, and flush with what holds it, no space above the first or below the last.
 *
 * ```html
 * <div [glzMarkdown]="reply.text" [streaming]="reply.streaming"></div>
 * ```
 */
@Component({
  selector: '[glzMarkdown]',
  templateUrl: './markdown.component.html',
  styleUrl: './markdown.component.scss',
  // The blocks' HTML carries no view attributes, so its styles are scoped by this class.
  encapsulation: ViewEncapsulation.None,
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'glz-markdown glz-flow', '[attr.data-testid]': 'testId()' },
})
export class GlzMarkdownComponent {
  /** The markdown. */
  readonly text = input.required<string>({ alias: 'glzMarkdown' });
  /** Whether the text is still being written. */
  readonly streaming = input(false, { transform: booleanAttribute });
  /** The test hook for this element; its code blocks' copy buttons get `<testId>-copy`. */
  readonly testId = input<string | null>(null);
  private readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('markdown', GLZ_MARKDOWN_LABELS);

  private readonly highlight = signal<GlzHighlight | null>(null);
  private readonly rendered = linkedSignal<
    { text: string; streaming: boolean; highlight: GlzHighlight | null },
    GlzMarkdown
  >({
    source: () => ({
      text: this.text(),
      streaming: this.streaming(),
      highlight: this.highlight(),
    }),
    computation: ({ text, ...options }, previous) =>
      renderMarkdown(text, { ...options, labels: this.words }, previous?.value ?? null),
  });
  protected readonly blocks = computed(() => this.rendered().blocks);

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private readonly app = inject(ApplicationRef);
  private readonly injector = inject(EnvironmentInjector);
  /** The copy buttons, by the code block head each sits in. */
  private readonly buttons = new Map<Element, ComponentRef<GlzCopyButtonComponent>>();
  /** The languages already asked for. */
  private readonly requested = new Set<string>();

  constructor() {
    let alive = true;
    inject(DestroyRef).onDestroy(() => {
      alive = false;
      for (const button of this.buttons.values()) {
        button.destroy();
      }
    });
    effect(() => {
      const wanted = this.rendered().languages.filter((language) => !this.requested.has(language));
      if (wanted.length === 0) {
        return;
      }
      for (const language of wanted) {
        this.requested.add(language);
      }
      void loadHighlighter(wanted)
        .then((highlight) => {
          if (alive && highlight !== null) {
            this.highlight.set(highlight);
          }
        })
        // Without highlighting, code shows plain.
        .catch(() => undefined);
    });
    afterRenderEffect(() => {
      this.blocks();
      this.placeButtons();
    });
  }

  /**
   * Gives each code block's head a copy button, and lets go of those whose block was redone:
   * its HTML was replaced, so the button left the page with it.
   */
  private placeButtons(): void {
    for (const [head, button] of this.buttons) {
      if (!this.host.contains(head)) {
        button.destroy();
        this.buttons.delete(head);
      }
    }
    for (const head of this.host.querySelectorAll('.glz-markdown-code-head')) {
      if (this.buttons.has(head)) {
        continue;
      }
      const code = head.parentElement?.querySelector('code')?.textContent ?? '';
      const element = this.host.ownerDocument.createElement('glz-copy-button');
      head.append(element);
      const button = createComponent(GlzCopyButtonComponent, {
        environmentInjector: this.injector,
        hostElement: element,
      });
      button.setInput('text', code);
      button.setInput('label', this.words.copyCode);
      button.setInput('testId', this.ids().copy);
      // Checked with the page; destroying it detaches it again.
      this.app.attachView(button.hostView);
      this.buttons.set(head, button);
    }
  }
}
