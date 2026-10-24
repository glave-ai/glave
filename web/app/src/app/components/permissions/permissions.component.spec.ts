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

  it('offers the presets the server lets a thread choose, showing the server’s at first', async () => {
    const fixture = await render(SERVER);
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('Permissions:');
    expect(trigger(root)?.textContent).toContain('Workspace');
    await expectAccessible(root);
    fixture.componentRef.setInput('current', 'read-only');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Read-only');
  });

  it('says what each means in a plain line, and takes the person’s pick', async () => {
    const fixture = await render(SERVER);
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    expect(options().map((option) => option.textContent)).toEqual([
      expect.stringContaining('Read-only'),
      expect.stringContaining('Workspace'),
      expect.stringContaining('Full access'),
    ]);
    expect(options()[0]?.textContent).toContain('asks before changing your files');
    expect(options()[1]?.textContent).toContain('asks before risky actions');
    expect(options()[2]?.textContent).toContain('for work you fully trust');
    await expectAccessible(document.body);
    options()[2]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('full-access');
  });

  it('offers only what the server allows', async () => {
    const fixture = await render({ ...SERVER, presets: ['read-only', 'workspace'] });
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    expect(options().map((option) => option.textContent)).toEqual([
      expect.stringContaining('Read-only'),
      expect.stringContaining('Workspace'),
    ]);
  });

  it('gives the server’s sentence where Workspace asks as Read-only does', async () => {
    const unsandboxed =
      'This computer can’t run the sandbox, so Read-only and Workspace ask before every change and command.';
    const fixture = await render({ ...SERVER, unsandboxed });
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    expect(options()[1]?.textContent).toContain(unsandboxed);
    expect(options()[1]?.textContent).not.toContain('risky');
  });

  it('shows the one preset the server allows, read-only', async () => {
    const fixture = await render({ ...SERVER, permissions: 'read-only', presets: ['read-only'] });
    const root: HTMLElement = fixture.nativeElement;
    const shown = root.querySelector('[data-testid="permissions"]');
    expect(shown?.textContent).toContain('Read-only');
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
