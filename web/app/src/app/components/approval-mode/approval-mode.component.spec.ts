import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { ApprovalModeComponent } from './approval-mode.component';

async function render(
  answer: object,
): Promise<ReturnType<typeof TestBed.createComponent<ApprovalModeComponent>>> {
  TestBed.configureTestingModule({
    providers: provideFakeSession(new FakeSession().answer('server/discover', answer)),
  });
  const fixture = TestBed.createComponent(ApprovalModeComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture;
}

function trigger(root: HTMLElement): HTMLButtonElement | null {
  return root.querySelector<HTMLButtonElement>('[data-testid="approval-mode-trigger"]');
}

describe('ApprovalModeComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('offers the modes the server lets a thread choose, showing the server’s at first', async () => {
    const fixture = await render(SERVER);
    const root: HTMLElement = fixture.nativeElement;
    expect(trigger(root)?.textContent).toContain('Approvals:');
    expect(trigger(root)?.textContent).toContain('Auto');
    await expectAccessible(root);
    fixture.componentRef.setInput('current', 'ask');
    await fixture.whenStable();
    expect(trigger(root)?.textContent).toContain('Ask');
  });

  it('takes the person’s pick', async () => {
    const fixture = await render(SERVER);
    trigger(fixture.nativeElement as HTMLElement)?.click();
    await fixture.whenStable();
    const items = document.querySelectorAll<HTMLElement>('[data-testid="approval-mode-option"]');
    expect(Array.from(items, (item) => item.textContent)).toEqual([
      expect.stringContaining('Ask'),
      expect.stringContaining('Auto'),
    ]);
    items[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.value()).toBe('ask');
  });

  it('shows the one mode the policy allows, read-only', async () => {
    const fixture = await render({ ...SERVER, approval: 'ask', approvals: ['ask'] });
    const root: HTMLElement = fixture.nativeElement;
    const mode = root.querySelector('[data-testid="approval-mode"]');
    expect(mode?.textContent).toContain('Ask');
    expect(mode?.getAttribute('title')).toContain('only mode');
    expect(root.querySelector('button')).toBeNull();
    await expectAccessible(root);
  });

  it('shows nothing until the server says', async () => {
    const fixture = await render({});
    expect(
      (fixture.nativeElement as HTMLElement).querySelector('[data-testid="approval-mode"]'),
    ).toBeNull();
  });
});
