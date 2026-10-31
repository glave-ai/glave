import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import type { Filters, Precision, Sort } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCheckboxComponent,
  GlzSegmentedComponent,
  type GlzOption,
} from '@glave/glaze';

import { type Place, countryName, placeCountries, placeOf } from '../../lib/serving';
import { FiltersService } from '../../services/filters.service';
import { ModelsService } from '../../services/models.service';

/** The precisions the picker offers, by the least each keeps. */
type Least = 'any' | Extract<Precision, 'eight_bit' | 'full'>;

/** A country a provider of the server's models is based in, as a checkbox shows it. */
interface Country {
  readonly code: string;
  readonly name: string;
  readonly checked: boolean;
}

/**
 * The person's filters among the providers a router passes their calls on to, at the top of
 * the model picker: which order the router prefers them in, the least precision, where they
 * may be based (a few places, or countries chosen one by one among those the providers are
 * based in), and only providers that keep nothing. Each choice is kept by the server at once
 * and holds for every call of their work; the list below shows what it leaves.
 */
@Component({
  selector: 'app-model-filters',
  imports: [GlzButtonComponent, GlzCheckboxComponent, GlzSegmentedComponent],
  templateUrl: './model-filters.component.html',
  styleUrl: './model-filters.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ModelFiltersComponent {
  protected readonly copy = {
    sort: $localize`:@@modelFilters.sort:Sort`,
    precision: $localize`:@@modelFilters.precision:Precision`,
    place: $localize`:@@modelFilters.place:Based in`,
    countries: $localize`:@@modelFilters.countries:Choose countries`,
    data: $localize`:@@modelFilters.data:Only providers that keep nothing`,
    retry: $localize`:@@modelFilters.retry:Try again`,
  } as const;
  protected readonly sorts: readonly GlzOption[] = [
    { value: 'price', label: $localize`:@@modelFilters.price:Lowest price` },
    { value: 'speed', label: $localize`:@@modelFilters.speed:Fastest` },
  ];
  protected readonly precisions: readonly GlzOption[] = [
    { value: 'any', label: $localize`:@@modelFilters.any:Any` },
    { value: 'eight_bit', label: $localize`:@@modelFilters.eightBit:8-bit or better` },
    { value: 'full', label: $localize`:@@modelFilters.full:Full only` },
  ];
  protected readonly places: readonly GlzOption[] = [
    { value: 'anywhere', label: $localize`:@@modelFilters.anywhere:Anywhere` },
    { value: 'us', label: $localize`:@@modelFilters.us:US only` },
    { value: 'us-europe', label: $localize`:@@modelFilters.usEurope:US and Europe` },
  ];
  private readonly service = inject(FiltersService);
  private readonly models = inject(ModelsService);
  private readonly locale = inject(LOCALE_ID);
  private readonly filters = this.service.filters;
  /** Whether the person opened the countries to choose them one by one. */
  private readonly opened = signal(false);

  protected readonly problem = this.service.problem;
  /** Whether the server said what they are, so they may be chosen. */
  protected readonly known = this.service.known;
  /** The order chosen, none while the router keeps its own balance. */
  protected readonly sort = computed<Sort | null>(() => this.filters().sort ?? null);
  protected readonly precision = computed<Least | null>(() => {
    const least = this.filters().precision ?? 'unstated';
    if (least === 'unstated') {
      return 'any';
    }
    return least === 'eight_bit' || least === 'full' ? least : null;
  });
  private readonly place = computed<Place>(() =>
    placeOf(this.filters().based_in ?? [], this.service.europe()),
  );
  /** The place chosen among the few offered, or `null` for countries chosen one by one. */
  protected readonly shownPlace = computed(() => {
    const place = this.place();
    return place === 'countries' ? null : place;
  });
  protected readonly keepsNothing = computed(() => this.filters().keeps_nothing ?? false);
  protected readonly choosingCountries = computed(
    () => this.opened() || this.place() === 'countries',
  );
  protected readonly offerCountries = computed(() => !this.choosingCountries());
  private readonly countryList = viewChild<ElementRef<HTMLElement>>('countryList');
  private readonly injector = inject(Injector);
  /** Every country a provider of the server's models is based in, by name. */
  protected readonly countries = computed<readonly Country[]>(() => {
    const chosen = this.filters().based_in ?? [];
    const codes = new Set(
      this.models
        .models()
        .flatMap((model) => model.endpoints ?? [])
        .flatMap((endpoint) => (endpoint.based_in ? [endpoint.based_in] : [])),
    );
    for (const code of chosen) {
      codes.add(code);
    }
    return [...codes]
      .map((code) => ({
        code,
        name: countryName(code, this.locale),
        checked: chosen.includes(code),
      }))
      .sort((one, other) => one.name.localeCompare(other.name, this.locale));
  });

  constructor() {
    void this.service.load();
    void this.models.load();
  }

  protected chooseSort(sort: string | null): void {
    if (sort === 'price' || sort === 'speed') {
      this.keep({ sort });
    }
  }

  protected choosePrecision(least: string | null): void {
    if (least === 'any') {
      this.keep({ precision: null });
    } else if (least === 'eight_bit' || least === 'full') {
      this.keep({ precision: least });
    }
  }

  protected choosePlace(place: string | null): void {
    if (place === 'anywhere' || place === 'us' || place === 'us-europe') {
      this.opened.set(false);
      this.keep({ based_in: placeCountries(place, this.service.europe()) });
    }
  }

  /** Shows the countries, focus moving to the first, as the button that showed them goes. */
  protected openCountries(): void {
    this.opened.set(true);
    afterNextRender(
      () => {
        this.countryList()?.nativeElement.querySelector<HTMLElement>('input')?.focus();
      },
      { injector: this.injector },
    );
  }

  protected retry(): void {
    void this.service.load();
  }

  protected chooseCountry(code: string, checked: boolean): void {
    // Once used, the countries stay, so the box never goes from under the person's focus.
    this.opened.set(true);
    const chosen = (this.filters().based_in ?? []).filter((each) => each !== code);
    this.keep({ based_in: checked ? [...chosen, code] : chosen });
  }

  protected chooseKeepsNothing(checked: boolean): void {
    this.keep({ keeps_nothing: checked });
  }

  private keep(change: Partial<Filters>): void {
    void this.service.choose({ ...this.filters(), ...change });
  }
}
