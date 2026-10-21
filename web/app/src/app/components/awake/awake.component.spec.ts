import { TestBed } from '@angular/core/testing';
import { ApiError, type AwakeInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AwakeComponent } from './awake.component';

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<AwakeComponent>>;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(AwakeComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function find<T extends HTMLElement>(root: ParentNode, testId: string): T | null {
  return root.querySelector<T>(`[data-testid="${testId}"]`);
}

/** A fake server that keeps the computer awake as it is told. */
function choosing(on: boolean): FakeSession {
  let awake: AwakeInfo = { kind: 'chosen', on };
  return new FakeSession()
    .answer('awake/get', () => ({ awake }))
    .answer('awake/set', (params) => {
      awake = { kind: 'chosen', on: (params as { on: boolean }).on };
      return { awake };
    });
}

describe('AwakeComponent', () => {
  it('shows the server’s choice as a switch, and sends the person’s', async () => {
    const fake = choosing(true);
    const { root, fixture } = await render(fake);
    expect(find(root, 'awake')?.textContent).toContain('This computer');
    const box = find<HTMLInputElement>(root, 'awake-on-input');
    expect(box?.checked).toBe(true);
    expect(find(root, 'awake')?.textContent).toContain('closing the lid still puts it to sleep');
    await expectAccessible(root);
    box?.click();
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(fake.paramsOf('awake/set')).toEqual([{ on: false }]);
    expect(find<HTMLInputElement>(root, 'awake-on-input')?.checked).toBe(false);
  });

  it('goes back to what the server said when it refuses, and says why', async () => {
    const fake = choosing(true).answer(
      'awake/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'your choice could not be kept' }),
    );
    const { root, fixture } = await render(fake);
    find<HTMLInputElement>(root, 'awake-on-input')?.click();
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(find(root, 'awake-problem')?.textContent).toBe('Your choice could not be kept.');
    expect(find<HTMLInputElement>(root, 'awake-on-input')?.checked).toBe(true);
  });

  it('says why where the computer cannot be kept awake', async () => {
    const reason = "This computer can't be kept awake while glave works.";
    const fake = new FakeSession().answer('awake/get', { awake: { kind: 'cannot', reason } });
    const { root } = await render(fake);
    expect(find(root, 'awake-cannot')?.textContent).toBe(reason);
    expect(find(root, 'awake-on')).toBeNull();
    await expectAccessible(root);
  });

  it('shows nothing on a server, which does not sleep', async () => {
    const fake = new FakeSession().answer('awake/get', { awake: { kind: 'never' } });
    const { root } = await render(fake);
    expect(find(root, 'awake')).toBeNull();
    expect(getComputedStyle(root).display).toBe('contents');
  });
});
