import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { AppearanceService, PLAIN } from './appearance.service';
import { ChangesService } from './changes.service';
import { INITIAL_SESSION, SessionService } from './session.service';

const SHADES = { light: '#0f766e', dark: '#59c5b8' };
const TEAL = { color: { color: '#0f766e', adjust: true } };

function root(name: string): string {
  return document.documentElement.style.getPropertyValue(name);
}

async function service(fake: FakeSession): Promise<AppearanceService> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const appearance = TestBed.inject(AppearanceService);
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  return appearance;
}

describe('AppearanceService', () => {
  afterEach(() => {
    localStorage.removeItem('glave-accent');
    document.documentElement.removeAttribute('style');
  });

  it('draws what the server says on the root, once for each of light and dark', async () => {
    const fake = new FakeSession().answer('appearance/get', { chosen: TEAL, shades: SHADES });
    const appearance = await service(fake);
    expect(root('--glz-accent-light')).toBe(SHADES.light);
    expect(root('--glz-accent-dark')).toBe(SHADES.dark);
    expect(appearance.value()).toBe('#0f766e');
    expect(appearance.adjust()).toBe(true);
    expect(JSON.parse(localStorage.getItem('glave-accent') ?? 'null')).toEqual(SHADES);
  });

  it('leaves Glaze’s plain default where there is no accent', async () => {
    localStorage.setItem('glave-accent', JSON.stringify(SHADES));
    const appearance = await service(new FakeSession().answer('appearance/get', { chosen: PLAIN }));
    expect(root('--glz-accent-light')).toBe('');
    expect(appearance.value()).toBe(PLAIN);
    expect(localStorage.getItem('glave-accent')).toBeNull();
  });

  it('is drawn in what this browser remembers before the server answers, nothing else', async () => {
    localStorage.setItem('glave-accent', JSON.stringify(SHADES));
    const fake = new FakeSession();
    fake.held('appearance/get');
    await service(fake);
    expect(root('--glz-accent-light')).toBe(SHADES.light);

    TestBed.resetTestingModule();
    document.documentElement.removeAttribute('style');
    localStorage.setItem('glave-accent', JSON.stringify({ light: 'url(evil)', dark: '#000000' }));
    const other = new FakeSession();
    other.held('appearance/get');
    await service(other);
    expect(root('--glz-accent-light')).toBe('');
  });

  it('reads again when the server says it changed elsewhere', async () => {
    const fake = new FakeSession().answer('appearance/get', { chosen: PLAIN });
    await service(fake);
    fake.answer('appearance/get', { chosen: TEAL, shades: SHADES });
    TestBed.inject(ChangesService).tell('appearance');
    TestBed.tick();
    await new Promise((resolve) => setTimeout(resolve));
    expect(root('--glz-accent-light')).toBe(SHADES.light);
  });

  it('shows the last of quick choices, and puts back what was kept, with why, when refused', async () => {
    const waiting: ((answer: object) => void)[] = [];
    const fake = new FakeSession().answer('appearance/get', { chosen: PLAIN }).answer(
      'appearance/set',
      () =>
        new Promise((resolve) => {
          waiting.push(resolve);
        }),
    );
    const appearance = await service(fake);
    const first = appearance.choose('#111111', true);
    const second = appearance.choose('#0f766e', true);
    expect(appearance.value()).toBe('#0f766e');
    await vi.waitFor(() => {
      expect(waiting).toHaveLength(1);
    });
    waiting[0]?.({ chosen: { color: { color: '#111111', adjust: true } }, shades: SHADES });
    await first;
    expect(appearance.value()).toBe('#0f766e');
    await vi.waitFor(() => {
      expect(waiting).toHaveLength(2);
    });
    waiting[1]?.({ chosen: TEAL, shades: SHADES });
    await second;
    expect(fake.paramsOf('appearance/set')).toHaveLength(2);

    fake.answer(
      'appearance/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
    );
    await appearance.choose('#222222', true);
    expect(appearance.value()).toBe('#0f766e');
    expect(appearance.problem()).toContain('The store is not answering.');
  });

  it('never loses No accent chosen while a color is on its way', async () => {
    const fake = new FakeSession()
      .answer('appearance/get', { chosen: PLAIN })
      .answer('appearance/set', (params) => {
        const { accent } = params as { readonly accent: unknown };
        return accent === PLAIN ? { chosen: PLAIN } : { chosen: TEAL, shades: SHADES };
      });
    const appearance = await service(fake);
    const color = appearance.choose('#0f766e', true);
    await Promise.all([color, appearance.choose(PLAIN, true)]);
    expect(fake.paramsOf('appearance/set').at(-1)).toEqual({ accent: PLAIN });
    expect(appearance.value()).toBe(PLAIN);
    expect(root('--glz-accent-light')).toBe('');
  });

  it('keeps what this browser remembers while the app starts signed out', () => {
    localStorage.setItem('glave-accent', JSON.stringify(SHADES));
    const fake = new FakeSession();
    TestBed.configureTestingModule({
      providers: [...provideFakeSession(fake), { provide: INITIAL_SESSION, useValue: null }],
    });
    TestBed.inject(AppearanceService);
    TestBed.tick();
    expect(root('--glz-accent-light')).toBe(SHADES.light);
    expect(localStorage.getItem('glave-accent')).not.toBeNull();
    expect(fake.paramsOf('appearance/get')).toEqual([]);
  });

  it('signing out takes the accent away, and an answer still on its way never brings it back', async () => {
    const fake = new FakeSession().answer('appearance/get', { chosen: TEAL, shades: SHADES });
    const appearance = await service(fake);
    expect(root('--glz-accent-light')).toBe(SHADES.light);
    const give = fake.held('appearance/set');
    const late = appearance.choose('#222222', true);
    await vi.waitFor(() => {
      expect(fake.paramsOf('appearance/set')).toHaveLength(1);
    });
    await TestBed.inject(SessionService).signOut();
    TestBed.tick();
    expect(root('--glz-accent-light')).toBe('');
    expect(appearance.value()).toBe(PLAIN);
    expect(localStorage.getItem('glave-accent')).toBeNull();
    give({ chosen: TEAL, shades: SHADES });
    await late;
    expect(root('--glz-accent-light')).toBe('');
    expect(localStorage.getItem('glave-accent')).toBeNull();
  });
});
