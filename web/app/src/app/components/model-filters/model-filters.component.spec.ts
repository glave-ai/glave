import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Filters } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ROUTED } from '../../testing/fixtures';
import { ModelFiltersComponent } from './model-filters.component';

const EUROPE = ['DE', 'FR', 'NL'];

async function render(
  filters: Filters = {},
): Promise<{ root: HTMLElement; fake: FakeSession; settle: () => Promise<void> }> {
  const fake = new FakeSession()
    .answer('filters/get', { filters, europe: EUROPE })
    .answer('filters/set', (params) => ({
      filters: (params as { readonly filters: Filters }).filters,
      europe: EUROPE,
    }))
    .answer('model/list', { models: ROUTED });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(ModelFiltersComponent);
  const settle = async (): Promise<void> => {
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  };
  await settle();
  return { root: fixture.nativeElement as HTMLElement, fake, settle };
}

/** The segment of `group` labelled `label`. */
function segment(root: HTMLElement, group: string, label: string): HTMLInputElement | null {
  const options = root.querySelectorAll<HTMLLabelElement>(`[data-testid="${group}-option"]`);
  const found = Array.from(options).find((option) => option.textContent.includes(label));
  return found?.querySelector('input') ?? null;
}

describe('ModelFiltersComponent', () => {
  it('shows the person’s filters: no order, any precision and anywhere until they choose', async () => {
    const { root } = await render();
    expect(segment(root, 'model-filters-sort', 'Lowest price')?.checked).toBe(false);
    expect(segment(root, 'model-filters-sort', 'Fastest')?.checked).toBe(false);
    expect(segment(root, 'model-filters-precision', 'Any')?.checked).toBe(true);
    expect(segment(root, 'model-filters-place', 'Anywhere')?.checked).toBe(true);
    expect(root.querySelector('[data-testid="model-filters-country-list"]')).toBeNull();
    await expectAccessible(root);
  });

  it('keeps each choice with the server: an order, a precision, a place, keeping nothing', async () => {
    const { root, fake, settle } = await render();
    segment(root, 'model-filters-sort', 'Fastest')?.click();
    await settle();
    segment(root, 'model-filters-precision', '8-bit or better')?.click();
    await settle();
    segment(root, 'model-filters-place', 'US and Europe')?.click();
    await settle();
    root
      .querySelector<HTMLInputElement>('[data-testid="model-filters-keeps-nothing"] input')
      ?.click();
    await settle();
    expect(fake.paramsOf('filters/set').at(-1)).toEqual({
      filters: {
        sort: 'speed',
        precision: 'eight_bit',
        based_in: ['US', ...EUROPE],
        keeps_nothing: true,
      },
    });
  });

  it('chooses countries one by one among those the providers are based in', async () => {
    const { root, fake, settle } = await render();
    root.querySelector<HTMLButtonElement>('[data-testid="model-filters-countries"]')?.click();
    await settle();
    const countries = root.querySelectorAll<HTMLElement>('[data-testid="model-filters-country"]');
    expect(Array.from(countries, (country) => country.textContent.trim())).toEqual([
      'China',
      'United States',
    ]);
    countries[1]?.querySelector('input')?.click();
    await settle();
    expect(fake.paramsOf('filters/set').at(-1)).toEqual({ filters: { based_in: ['US'] } });
    await expectAccessible(root);
  });

  it('shows countries chosen one by one as no place of the few', async () => {
    const { root } = await render({ based_in: ['CN'] });
    expect(segment(root, 'model-filters-place', 'Anywhere')?.checked).toBe(false);
    expect(root.querySelector('[data-testid="model-filters-country-list"]')).not.toBeNull();
  });

  it('offers none to choose until they are read, says why, and reads them again', async () => {
    const fake = new FakeSession()
      .answer(
        'filters/get',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
      )
      .answer('model/list', { models: ROUTED });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(ModelFiltersComponent);
    const root = fixture.nativeElement as HTMLElement;
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="model-filters"]')).toBeNull();
    expect(root.querySelector('[data-testid="model-filters-unread"]')?.textContent).toContain(
      'The store is not answering.',
    );
    await expectAccessible(root);
    fake.answer('filters/get', { filters: {}, europe: EUROPE });
    root.querySelector<HTMLButtonElement>('[data-testid="model-filters-retry"]')?.click();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-testid="model-filters"]')).not.toBeNull();
    });
  });
});
