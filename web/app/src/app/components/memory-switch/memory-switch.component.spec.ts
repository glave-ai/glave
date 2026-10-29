import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type RememberingInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MemorySwitchComponent } from './memory-switch.component';

async function settle(fixture: { whenStable: () => Promise<unknown> }): Promise<void> {
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<MemorySwitchComponent>>;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(MemorySwitchComponent);
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function box(root: ParentNode): HTMLInputElement | null {
  return root.querySelector<HTMLInputElement>('[data-testid="memory-switch-on-input"]');
}

/** A fake server that keeps the person's choice as it is told. */
function choosing(on: boolean): FakeSession {
  let remembering: RememberingInfo = { kind: 'chosen', on };
  return new FakeSession()
    .answer('memory/get', () => ({ remembering }))
    .answer('memory/set', (params) => {
      remembering = { kind: 'chosen', on: (params as { on: boolean }).on };
      return { remembering };
    });
}

describe('MemorySwitchComponent', () => {
  it('shows the server’s choice as a switch, and sends the person’s', async () => {
    const fake = choosing(true);
    const { root, fixture } = await render(fake);
    expect(root.textContent).toContain('Let glave remember');
    expect(box(root)?.checked).toBe(true);
    expect(box(root)?.disabled).toBe(false);
    await expectAccessible(root);
    box(root)?.click();
    await settle(fixture);
    expect(fake.paramsOf('memory/set')).toEqual([{ on: false }]);
    expect(box(root)?.checked).toBe(false);
  });

  it('shows off and unchangeable with the server’s sentence where it is not the person’s', async () => {
    const fake = new FakeSession().answer('memory/get', {
      remembering: { kind: 'never', reason: 'memory is off for this server' },
    });
    const { root } = await render(fake);
    expect(box(root)?.checked).toBe(false);
    expect(box(root)?.disabled).toBe(true);
    expect(root.textContent).toContain('Memory is off for this server');
    await expectAccessible(root);
  });

  it('goes back to what the server said when it refuses, and says why', async () => {
    const fake = choosing(true).answer(
      'memory/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'your choice could not be kept' }),
    );
    const { root, fixture } = await render(fake);
    box(root)?.click();
    await settle(fixture);
    expect(box(root)?.checked).toBe(true);
    expect(root.querySelector('[data-testid="memory-switch-problem"]')?.textContent).toContain(
      'Your choice could not be kept',
    );
  });
});
