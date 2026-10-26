import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type SearchInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { WebSearchComponent } from './web-search.component';

const DETAIL = 'Uses Exa, free. Your searches go to Exa.';

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<WebSearchComponent>>;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(WebSearchComponent);
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

function find<T extends HTMLElement>(root: ParentNode, testId: string): T | null {
  return root.querySelector<T>(`[data-testid="${testId}"]`);
}

/** A fake server that turns web search on and off as it is told. */
function choosing(on: boolean): FakeSession {
  const said = (now: boolean): SearchInfo =>
    now
      ? {
          kind: 'chosen',
          on: true,
          detail: DETAIL,
          usage: { calls: 412, sentence: '412 searches this month' },
        }
      : { kind: 'chosen', on: false, detail: DETAIL };
  let search = said(on);
  return new FakeSession()
    .answer('search/get', () => ({ search }))
    .answer('search/set', (params) => {
      search = said((params as { on: boolean }).on);
      return { search };
    });
}

describe('WebSearchComponent', () => {
  it('shows the server’s word as a switch, and turns it on and off', async () => {
    const fake = choosing(false);
    const { root, fixture } = await render(fake);
    expect(find(root, 'web-search')?.textContent).toContain('Web search');
    expect(find(root, 'web-search')?.textContent).toContain(DETAIL);
    const box = find<HTMLInputElement>(root, 'web-search-on-input');
    expect(box?.checked).toBe(false);
    expect(find(root, 'web-search-usage')).toBeNull();
    await expectAccessible(root);

    box?.click();
    await settle(fixture);
    expect(fake.paramsOf('search/set')).toEqual([{ on: true }]);
    expect(find<HTMLInputElement>(root, 'web-search-on-input')?.checked).toBe(true);
    expect(find(root, 'web-search-usage')?.textContent).toBe('412 searches this month');
    await expectAccessible(root);

    find<HTMLInputElement>(root, 'web-search-on-input')?.click();
    await settle(fixture);
    expect(fake.paramsOf('search/set')).toEqual([{ on: true }, { on: false }]);
    expect(find<HTMLInputElement>(root, 'web-search-on-input')?.checked).toBe(false);
  });

  it('goes back to what the server said when it refuses, and says why', async () => {
    const fake = choosing(true).answer(
      'search/set',
      new ApiError({
        code: ErrorCode.NOT_AVAILABLE,
        message:
          'you set up web search yourself in your own settings, so glave leaves it there: change it there',
      }),
    );
    const { root, fixture } = await render(fake);
    find<HTMLInputElement>(root, 'web-search-on-input')?.click();
    await settle(fixture);
    expect(find(root, 'web-search-problem')?.textContent).toBe(
      'You set up web search yourself in your own settings, so glave leaves it there: change it there.',
    );
    expect(find<HTMLInputElement>(root, 'web-search-on-input')?.checked).toBe(true);
    await expectAccessible(root);
  });

  it('shows nothing where web search is not the person’s to choose', async () => {
    const { root } = await render(new FakeSession());
    expect(find(root, 'web-search')).toBeNull();
    expect(getComputedStyle(root).display).toBe('contents');
  });
});
