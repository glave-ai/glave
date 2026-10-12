import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { ApprovalModeComponent } from './approval-mode.component';

async function render(answer: object): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: provideFakeSession(new FakeSession().answer('server/discover', answer)),
  });
  const fixture = TestBed.createComponent(ApprovalModeComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('ApprovalModeComponent', () => {
  it('shows the server’s mode, read-only, and why it cannot change here', async () => {
    const root = await render(SERVER);
    const mode = root.querySelector('[data-testid="approval-mode"]');
    expect(mode?.textContent).toContain('Approvals:');
    expect(mode?.textContent).toContain('Auto');
    expect(mode?.getAttribute('title')).toContain('choosing it per thread comes later');
    expect(root.querySelector('button')).toBeNull();
    await expectAccessible(root);
  });

  it('shows ask mode', async () => {
    const root = await render({ ...SERVER, approval: 'ask' });
    expect(root.querySelector('[data-testid="approval-mode"]')?.textContent).toContain('Ask');
  });

  it('shows nothing until the server says', async () => {
    const root = await render({});
    expect(root.querySelector('[data-testid="approval-mode"]')).toBeNull();
  });
});
