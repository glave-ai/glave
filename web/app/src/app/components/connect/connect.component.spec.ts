import { Location } from '@angular/common';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type ProviderInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { NAVIGATION } from '../../services/back.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MODELS } from '../../testing/fixtures';
import { ConnectComponent } from './connect.component';

const ACCOUNT: ProviderInfo = {
  id: '01900000-0000-7000-8000-000000000301',
  kind: 'compatible',
  url: 'http://localhost:11434/v1',
  models: ['gemma4:12b'],
};

function find<T extends Element>(root: HTMLElement, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

function type(root: HTMLElement, testId: string, value: string): void {
  const field = find<HTMLInputElement>(root, testId);
  field.value = value;
  field.dispatchEvent(new Event('input'));
}

type Fixture = ComponentFixture<ConnectComponent>;

async function setUp(fake: FakeSession, kind?: string): Promise<Fixture> {
  TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
  vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
  const fixture = TestBed.createComponent(ConnectComponent);
  if (kind !== undefined) {
    fixture.componentRef.setInput('kind', kind);
  }
  await fixture.whenStable();
  return fixture;
}

/** Sends the form, and waits for the check and what follows it. */
async function submit(fixture: Fixture): Promise<void> {
  const root = fixture.nativeElement as HTMLElement;
  find(root, 'connect').dispatchEvent(new Event('submit', { cancelable: true }));
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

describe('ConnectComponent', () => {
  it('offers the services by plain names, and a key field that never shows the key', async () => {
    const fixture = await setUp(new FakeSession().answer('model/list', { models: [] }));
    const root = fixture.nativeElement as HTMLElement;
    expect(find(root, 'connect-service').textContent).toContain('Another server');
    expect(root.textContent).not.toMatch(/config|toml|GLAVE_|--/u);
    expect(find<HTMLInputElement>(root, 'connect-key').type).toBe('password');
    expect(root.querySelector('[data-testid="connect-address"]')).toBeNull();
    expect(root.querySelector('[data-testid="connect-back"]')).toBeNull();
    await expectAccessible(root);
  });

  it('leads with OpenRouter and any OpenAI-compatible server, each said in a line', async () => {
    const fake = new FakeSession()
      .answer('model/list', { models: [] })
      .answer('provider/connect', { provider: { ...ACCOUNT, kind: 'openrouter', url: undefined } })
      .answer('provider/list', { providers: [] });
    const fixture = await setUp(fake);
    const root = fixture.nativeElement as HTMLElement;
    const radios = Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
    expect(radios.map((radio) => radio.value)).toEqual(['openrouter', 'openai', 'compatible']);
    expect(radios[0]?.checked).toBe(true);
    expect(find(root, 'connect-service').textContent).toContain('every major model');
    type(root, 'connect-key', 'router-key');
    await submit(fixture);
    expect(fake.paramsOf('provider/connect')).toEqual([{ kind: 'openrouter', key: 'router-key' }]);
  });

  it('still changes the key of an account with a service no longer offered', async () => {
    const fixture = await setUp(
      new FakeSession().answer('model/list', { models: MODELS }),
      'anthropic',
    );
    const root = fixture.nativeElement as HTMLElement;
    const checked = root.querySelector<HTMLInputElement>('input[type="radio"]:checked');
    expect(checked?.value).toBe('anthropic');
  });

  it('asks for an address for another server, and sends the key to be checked', async () => {
    let connected = false;
    const fake = new FakeSession()
      .answer('model/list', () => ({ models: connected ? MODELS : [] }))
      .answer('provider/connect', () => {
        connected = true;
        return { provider: ACCOUNT };
      })
      .answer('provider/list', { providers: [ACCOUNT] });
    const fixture = await setUp(fake, 'compatible');
    const root = fixture.nativeElement as HTMLElement;
    await fixture.whenStable();
    type(root, 'connect-address', ' http://localhost:11434/v1 ');
    type(root, 'connect-key', ' local-key ');
    await fixture.whenStable();
    await submit(fixture);
    expect(fake.paramsOf('provider/connect')).toEqual([
      { kind: 'compatible', key: 'local-key', url: 'http://localhost:11434/v1' },
    ]);
    expect(TestBed.inject(Router).navigateByUrl).toHaveBeenCalledWith('/');
  });

  it('goes back where the person came from when adding an account: on Back, and once kept', async () => {
    const fake = new FakeSession()
      .answer('model/list', { models: MODELS })
      .answer('provider/connect', { provider: { ...ACCOUNT, kind: 'openrouter', url: undefined } })
      .answer('provider/list', { providers: [] });
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        ...provideFakeSession(fake),
        { provide: NAVIGATION, useValue: { canGoBack: true } },
      ],
    });
    const back = vi.spyOn(TestBed.inject(Location), 'back').mockReturnValue(undefined);
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(ConnectComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    find<HTMLButtonElement>(root, 'connect-back').click();
    await fixture.whenStable();
    expect(back).toHaveBeenCalledOnce();
    type(root, 'connect-key', 'router-key');
    await submit(fixture);
    expect(fake.paramsOf('provider/connect')).toEqual([{ kind: 'openrouter', key: 'router-key' }]);
    expect(back).toHaveBeenCalledTimes(2);
    expect(navigate).not.toHaveBeenCalled();
  });

  it("starts from another server's address when it changes that server's key", async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        ...provideFakeSession(new FakeSession().answer('model/list', { models: MODELS })),
      ],
    });
    const fixture = TestBed.createComponent(ConnectComponent);
    fixture.componentRef.setInput('kind', 'compatible');
    fixture.componentRef.setInput('url', 'http://localhost:11434/v1');
    await fixture.whenStable();
    const root = fixture.nativeElement as HTMLElement;
    expect(find<HTMLInputElement>(root, 'connect-address').value).toBe('http://localhost:11434/v1');
  });

  it("says what to do when the key is refused, in the server's words", async () => {
    const refused = new ApiError({
      code: ErrorCode.PROVIDER,
      message: 'OpenAI did not accept that key: check that you copied all of it',
      data: { failure: 'refused' },
    });
    const fake = new FakeSession()
      .answer('model/list', { models: [] })
      .answer('provider/connect', refused);
    const fixture = await setUp(fake);
    const root = fixture.nativeElement as HTMLElement;
    await submit(fixture);
    expect(find(root, 'connect-problem').textContent).toBe('Paste your key first.');
    type(root, 'connect-key', 'wrong-key');
    await submit(fixture);
    expect(find(root, 'connect-problem').textContent).toContain('did not accept that key');
    await expectAccessible(root);
  });
});
