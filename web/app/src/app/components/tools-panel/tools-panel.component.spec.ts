import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { event, task } from '../../testing/fixtures';
import { ToolsPanelComponent } from './tools-panel.component';

const EDITED = [
  event(1, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'write',
    arguments: { json: { path: 'a.md', content: 'x' } },
  }),
  event(2, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
];

describe('ToolsPanelComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the tasks and the files in tabs, remembers the tab, and closes', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: EDITED })
      .answer('tasks/list', { tasks: [task('t9', 'Nightly build')] });
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
    });
    await TestBed.inject(ThreadService).open('thread-1');
    const panel = TestBed.inject(ToolsPanelService);
    panel.open.set(true);
    const fixture = TestBed.createComponent(ToolsPanelComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const tabs = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="tools-tabs-tab"]'));
    expect(tabs.map((tab) => tab.textContent.trim())).toEqual(['Tasks', 'Files (1)']);
    expect(root.querySelectorAll('[data-testid="task"]')).toHaveLength(1);
    await expectAccessible(root);
    tabs[1]?.click();
    await fixture.whenStable();
    expect(panel.tab()).toBe('files');
    expect(root.querySelectorAll('[data-testid="file"]')).toHaveLength(1);
    root.querySelector<HTMLButtonElement>('[data-testid="tools-close"]')?.click();
    expect(panel.open()).toBe(false);
  });
});
