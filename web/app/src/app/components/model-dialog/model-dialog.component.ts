import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  model,
  output,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import type { EndpointInfo, ModelInfo, TokenPrice } from '@glave/client';
import {
  GlzButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzIconComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
} from '@glave/glaze';
import { Check } from 'lucide';

import { contextText, displayName, modelHint } from '../../lib/models';
import {
  type Choice,
  DOT,
  cheapest,
  choiceOf,
  fastest,
  hiddenEndpoints,
  hiddenLine,
  listed,
  priceLine,
  providerDetail,
  providersText,
  routed,
  shownEndpoints,
  sortedModels,
  speedLine,
  withChoice,
} from '../../lib/serving';
import { FiltersService } from '../../services/filters.service';
import { ModelsService } from '../../services/models.service';
import { ModelFiltersComponent } from '../model-filters/model-filters.component';

/** A model as the list shows it. */
interface Row {
  readonly id: string;
  readonly name: string;
  readonly line: string;
  readonly price: string;
  readonly speed: string;
  /** Whether it is the model in use. */
  readonly current: boolean;
}

/** One choice of who serves a model. */
interface Serving {
  readonly value: Choice;
  readonly label: string;
  readonly hint: string;
  readonly disabled: boolean;
}

/**
 * The model picker, where a router serves the server's models: a dialog to search them, the
 * person's filters among the router's providers above them (`app-model-filters`), and each
 * model the filters leave with its cheapest price and best speed among its providers, in the
 * order the filters prefer. A model no router serves is picked at once; one a router serves
 * opens who serves it: glave's choice by lowest price or speed, or each provider with where it
 * is based, its precision, what it keeps, its price and speed, and which the filters hide and
 * why. Using it keeps that choice with the filters, once the server has kept it, and picks the
 * model. `value` is bound two ways: the model picked, `null` until one is.
 */
@Component({
  selector: 'app-model-dialog',
  imports: [
    GlzButtonComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzIconComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
    ModelFiltersComponent,
  ],
  templateUrl: './model-dialog.component.html',
  styleUrl: './model-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ModelDialogComponent {
  /** Whether the dialog is open. */
  readonly open = model(false);
  /** The model in use, marked in the list, or `null` for none. */
  readonly current = input<string | null>(null);
  /** The model picked, or `null` until one is. */
  readonly value = model<string | null>(null);
  /** Told each model picked. */
  readonly picked = output<string>();

  protected readonly copy = {
    heading: $localize`:@@modelDialog.heading:Choose a model`,
    search: $localize`:@@modelDialog.search:Search models`,
    searchHint: $localize`:@@modelDialog.searchHint:Search by name or maker`,
    none: $localize`:@@modelDialog.none:No model matches these filters.`,
    noMatch: $localize`:@@modelDialog.noMatch:No model matches your search.`,
    current: $localize`:@@modelDialog.current:Chosen`,
    prices: $localize`:@@modelDialog.prices:Prices are per million tokens, in and out. No provider that trains on your chats ever serves glave.`,
    back: $localize`:@@modelDialog.back:All models`,
    who: $localize`:@@modelDialog.who:Who serves it`,
    showAll: $localize`:@@modelDialog.showAll:Show all`,
    hideThem: $localize`:@@modelDialog.hideThem:Hide them`,
    pickedOne: $localize`:@@modelDialog.pickedOne:Prices are per million tokens, in and out. If the provider you pick can’t serve it, glave says so rather than using another.`,
    cancel: $localize`:@@modelDialog.cancel:Back`,
  } as const;
  protected readonly icons = { current: Check } as const;
  private readonly models = inject(ModelsService);
  private readonly service = inject(FiltersService);
  private readonly locale = inject(LOCALE_ID);
  private readonly injector = inject(Injector);
  private readonly filters = this.service.filters;
  private readonly host = viewChild.required<ElementRef<HTMLElement>>('host');

  protected readonly query = signal('');
  /** The model whose providers are shown, or `null` for the list. */
  private readonly viewing = signal<string | null>(null);
  /** Whether the providers the filters hide are shown too. */
  protected readonly showingAll = signal(false);
  /** Who the person chose to serve the model shown, until they use it. */
  private readonly chosen = signal<Choice | null>(null);
  /** Why the filters could not be read or kept, until the next answer. */
  protected readonly problem = this.service.problem;
  /** Whether a choice of who serves the model shown is on its way to the server. */
  protected readonly busy = signal(false);

  protected readonly rows = computed<readonly Row[]>(() => {
    const query = this.query().trim().toLocaleLowerCase(this.locale);
    const found = this.models
      .models()
      .filter(listed)
      .filter((each) =>
        [each.name, each.maker ?? '', each.id].some((text) =>
          text.toLocaleLowerCase(this.locale).includes(query),
        ),
      );
    return sortedModels(found, this.filters().sort ?? 'price').map((each) => this.row(each));
  });
  /** Why the list is empty, or `''` while it is not. */
  protected readonly empty = computed(() => {
    if (this.rows().length > 0) {
      return '';
    }
    return this.query().trim() === '' ? this.copy.none : this.copy.noMatch;
  });
  protected readonly listing = computed(() => this.shown() === null);
  /** The model whose providers are shown, while one is. */
  protected readonly shown = computed(() => {
    const id = this.viewing();
    return this.models.models().find((each) => each.id === id) ?? null;
  });
  protected readonly shownName = computed(() => this.shown()?.name ?? '');
  protected readonly shownLine = computed(() => {
    const shown = this.shown();
    return shown === null ? '' : this.line(shown, false);
  });
  /** Which providers the filters hide, and why, or `''` when they hide none. */
  protected readonly hidden = computed(() => {
    const shown = this.shown();
    const hidden = shown === null ? [] : hiddenEndpoints(shown);
    return hidden.length === 0 ? '' : this.hiddenSentence(hidden);
  });
  protected readonly showAllLabel = computed(() =>
    this.showingAll() ? this.copy.hideThem : this.copy.showAll,
  );
  /**
   * Who serves the model shown: the person's choice here, else the filters'; none where that is
   * a provider the filters now hide.
   */
  protected readonly choice = computed<Choice | null>(() => {
    const shown = this.shown();
    if (shown === null) {
      return null;
    }
    const choice = this.chosen() ?? choiceOf(this.filters(), shown.id);
    const hidden = hiddenEndpoints(shown).some((each) => choice === `provider:${each.slug}`);
    return hidden ? null : choice;
  });
  protected readonly servings = computed<readonly Serving[]>(() => {
    const shown = this.shown();
    return shown === null ? [] : this.servingsOf(shown);
  });
  protected readonly use = computed(() => {
    const name = this.shownName();
    return $localize`:@@modelDialog.use:Use ${name}:MODEL:`;
  });

  constructor() {
    void this.models.load();
    void this.service.load();
    effect(() => {
      if (!this.open()) {
        untracked(() => {
          this.viewing.set(null);
          this.chosen.set(null);
          this.showingAll.set(false);
          this.query.set('');
        });
      }
    });
  }

  protected search(event: Event): void {
    this.query.set((event.target as HTMLInputElement).value);
  }

  /** Picks a model no router serves; opens who serves one a router does. */
  protected show(id: string): void {
    const chosen = this.models.models().find((each) => each.id === id);
    if (chosen !== undefined && routed(chosen)) {
      this.chosen.set(null);
      this.showingAll.set(false);
      this.viewing.set(id);
      this.focus('[data-testid="model-dialog-back"]');
    } else {
      this.pick(id);
    }
  }

  protected backToList(): void {
    const id = this.viewing();
    this.viewing.set(null);
    if (id !== null) {
      this.focus(`[data-model="${CSS.escape(id)}"]`);
    }
  }

  protected toggleAll(): void {
    this.showingAll.update((all) => !all);
  }

  protected choose(value: string | null): void {
    if (value !== null && (value.startsWith('sort:') || value.startsWith('provider:'))) {
      this.chosen.set(value as Choice);
    }
  }

  /**
   * Keeps who serves the model shown, as chosen, and picks it once the server has kept it; one
   * the server keeps not stays open, with why.
   */
  protected async useShown(): Promise<void> {
    const shown = this.shown();
    if (shown === null || this.busy()) {
      return;
    }
    const choice = this.choice();
    if (this.service.known() && choice !== choiceOf(this.filters(), shown.id)) {
      this.busy.set(true);
      const kept = await this.service.choose(withChoice(this.filters(), shown.id, choice));
      this.busy.set(false);
      if (!kept) {
        return;
      }
    }
    this.pick(shown.id);
  }

  private pick(id: string): void {
    this.value.set(id);
    this.picked.emit(id);
    this.open.set(false);
  }

  /** Moves focus to what `selector` finds in the dialog, once it is drawn. */
  private focus(selector: string): void {
    afterNextRender(
      () => {
        this.host().nativeElement.querySelector<HTMLElement>(selector)?.focus();
      },
      { injector: this.injector },
    );
  }

  private row(each: ModelInfo): Row {
    const current = each.id === this.current();
    if (!routed(each)) {
      const line = modelHint(each, this.locale);
      return { id: each.id, name: displayName(each), line, price: '', speed: '', current };
    }
    const shown = shownEndpoints(each);
    return {
      id: each.id,
      name: each.name,
      line: this.line(each, true),
      price: this.from(cheapest(shown)),
      speed: this.upTo(fastest(shown)),
      current,
    };
  }

  /** `from $0.12 in · $0.50 out`, for the cheapest provider's `price`; `''` for none. */
  private from(price: TokenPrice | null): string {
    if (price === null) {
      return '';
    }
    const least = priceLine(price, this.locale);
    return $localize`:@@modelDialog.from:from ${least}:PRICE:`;
  }

  /** `up to 210 tokens a second`, for the fastest provider's `speed`; `''` for none. */
  private upTo(speed: number | null): string {
    if (speed === null) {
      return '';
    }
    const most = speedLine(speed, this.locale);
    return $localize`:@@modelDialog.upTo:up to ${most}:SPEED:`;
  }

  /** `model`'s maker, context and, when asked, how many providers the filters leave. */
  private line(each: ModelInfo, counted: boolean): string {
    const context = each.context ?? null;
    return [
      each.maker ?? '',
      context === null ? '' : contextText(context, this.locale),
      counted ? providersText(shownEndpoints(each).length, this.locale) : '',
    ]
      .filter((part) => part !== '')
      .join(DOT);
  }

  private hiddenSentence(hidden: readonly EndpointInfo[]): string {
    return hiddenLine(hidden, this.locale);
  }

  /** Who may serve `each`: glave's choice by price or speed, then each provider once. */
  private servingsOf(each: ModelInfo): readonly Serving[] {
    const shown = shownEndpoints(each);
    const price = cheapest(shown);
    const speed = fastest(shown);
    const glave: readonly Serving[] = [
      {
        value: 'sort:price',
        label: $localize`:@@modelDialog.cheapest:Let glave choose: lowest price`,
        hint: [
          $localize`:@@modelDialog.cheapestHint:Whichever provider within your filters is cheapest when it’s asked`,
          this.from(price),
        ]
          .filter((part) => part !== '')
          .join(DOT),
        disabled: false,
      },
      {
        value: 'sort:speed',
        label: $localize`:@@modelDialog.fastest:Let glave choose: fastest`,
        hint: [
          $localize`:@@modelDialog.fastestHint:Whichever provider within your filters is quickest when it’s asked`,
          this.upTo(speed),
        ]
          .filter((part) => part !== '')
          .join(DOT),
        disabled: false,
      },
    ];
    const seen = new Set<string>();
    const providers = (each.endpoints ?? [])
      .filter((endpoint) => this.showingAll() || (endpoint.hidden_by ?? []).length === 0)
      .filter((endpoint) => {
        const fresh = !seen.has(endpoint.slug);
        seen.add(endpoint.slug);
        return fresh;
      })
      .map((endpoint) => {
        const hidden = (endpoint.hidden_by ?? []).length > 0;
        return {
          value: `provider:${endpoint.slug}` as Choice,
          label: endpoint.provider,
          hint: [
            providerDetail(endpoint, this.locale),
            endpoint.price ? priceLine(endpoint.price, this.locale) : '',
            endpoint.speed ? speedLine(endpoint.speed, this.locale) : '',
            hidden ? this.hiddenSentence([endpoint]) : '',
          ]
            .filter((part) => part !== '')
            .join(DOT),
          disabled: hidden,
        };
      });
    return [...glave, ...providers];
  }
}
