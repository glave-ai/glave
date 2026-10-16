import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { NavService } from '../../services/nav.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space } from '../../testing/fixtures';
import { WorkspaceComponent } from './workspace.component';

describe('WorkspaceComponent', () => {
  it('reads the spaces and frames the page with the side nav', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [space('p1', 'glave')] })
      .answer('thread/list', { threads: [] })
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [] });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(WorkspaceComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(fake.paramsOf('space/list')).toHaveLength(1);
    expect(root.querySelector('[data-testid="workspace-nav"] app-side-nav')).not.toBeNull();
    expect(root.querySelector('main#content')).not.toBeNull();
    expect(root.querySelector('[data-testid="needs-you"]')).not.toBeNull();
    await expectAccessible(root);
    TestBed.inject(NavService).toggle();
    await fixture.whenStable();
    expect(root.querySelector<HTMLElement>('[data-testid="workspace-nav"]')?.inert).toBe(true);
  });

  it('goes to the sign-in page when the server refuses the sign-in', async () => {
    const fake = new FakeSession()
      .answer('space/list', new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'expired' }))
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [] });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(WorkspaceComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(navigate).toHaveBeenCalledWith('/sign-in');
  });
});
