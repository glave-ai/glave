import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { PermissionsComponent } from './permissions.component';

async function render(
  answer: object,
): Promise<ReturnType<typeof TestBed.createComponent<PermissionsComponent>>> {
  TestBed.configureTestingModule({
    providers: provideFakeSession(new FakeSession().answer('server/discover', answer)),
  });
  const fixture = TestBed.createComponent(PermissionsComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture;
}

function trigger(root: HTMLElement): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>('[data-testid="permissions-trigger"]');
}

function options(): readonly HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-testid="permissions-option"]'));
}

describe('PermissionsComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('offers the modes the server lets a thread choose, showing the server’s at first', async () => {
    const fixture = await render(SERVER);
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('Mode:');
    expect(trigger(root)?.textContent).toContain('Auto');
    expect(trigger(root)?.textContent).not.toContain('Recommended');
    await expectAccessible(root);
    fixture.componentRef.setInput('current', 'ask');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Ask first');
  });

  it('lists Auto, recommended, then Ask first and Full access, each with its line', async () => {
    const fixture = await render(SERVER);
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    const labels = options().map((option) => option.querySelector('.label')?.textContent.trim());
    expect(labels).toEqual(['Auto · Recommended', 'Ask first', 'Full access']);
    const lines = options().map((option) => option.querySelector('.hint')?.textContent.trim());
    expect(lines).toEqual([
      'Works on its own in this project. Asks only before something risky.',
      'Asks you before every change.',
      'No limits: can change anything on this computer, and never asks.',
    ]);
    await expectAccessible(document.body);
    options()[2]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('full-access');
  });

  it('offers only what the server allows', async () => {
    const fixture = await render({ ...SERVER, presets: ['auto', 'ask'] });
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    expect(options().map((option) => option.textContent)).toEqual([
      expect.stringContaining('Auto'),
      expect.stringContaining('Ask first'),
    ]);
  });

  it('gives the server’s sentence where Auto asks before everything', async () => {
    const unsandboxed =
      'This computer can’t run the sandbox, so Auto and Ask first both ask before every change and command.';
    const fixture = await render({ ...SERVER, unsandboxed });
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    expect(options()[0]?.textContent).toContain(unsandboxed);
    expect(options()[0]?.textContent).not.toContain('risky');
  });

  it('shows the one mode the server allows, without a menu', async () => {
    const fixture = await render({ ...SERVER, permissions: 'ask', presets: ['ask'] });
    const root: HTMLElement = fixture.nativeElement;
    const shown = root.querySelector('[data-testid="permissions"]');
    expect(shown?.textContent).toContain('Ask first');
    expect(shown?.getAttribute('title')).toContain('only choice');
    expect(root.querySelector('button')).toBeNull();
    await expectAccessible(root);
  });

  it('shows nothing until the server says', async () => {
    const fixture = await render({});
    expect(
      (fixture.nativeElement as HTMLElement).querySelector('[data-testid="permissions"]'),
    ).toBeNull();
  });
});
