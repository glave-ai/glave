import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space } from '../../testing/fixtures';
import { WorkspaceComponent } from './workspace.component';

describe('WorkspaceComponent', () => {
  it('reads the spaces and frames the page with the side nav and top bar', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [space('p1', 'glave')] })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(WorkspaceComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(fake.paramsOf('space/list')).toHaveLength(1);
    expect(root.querySelector('app-spaces-nav')).not.toBeNull();
    expect(root.querySelector('[data-testid="workspace-tasks"]')?.getAttribute('aria-label')).toBe(
      'Tasks',
    );
    await expectAccessible(root);
  });
});
