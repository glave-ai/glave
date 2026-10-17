import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { CommandsService } from '../../services/commands.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event, task } from '../../testing/fixtures';
import { ToolsPanelComponent } from './tools-panel.component';

const EDITED = [
  event(1, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'write',
    arguments: { json: { path: 'a.md', content: 'x' } },
  }),
  event(2, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
  event(3, 'command.started', { command: 'cmd-1', script: 'npm run dev' }),
];

describe('ToolsPanelComponent', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows the tasks, the changes and the commands in tabs, remembers the tab, and closes as a sheet', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: EDITED })
      .answer('command/list', { commands: [command('cmd-1', 'npm run dev')] })
      .answer('schedule/list', { schedules: [] })
      .answer('tasks/list', { tasks: [task('t9', 'Nightly build')] });
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        ...provideFakeSession(fake),
        ThreadService,
        CommandsService,
        // A narrow screen: the panel is a sheet, with its own close button.
        { provide: BreakpointObserver, useValue: { observe: () => of({ matches: true }) } },
      ],
    });
    await TestBed.inject(ThreadService).open('thread-1');
    const panel = TestBed.inject(ToolsPanelService);
    panel.open.set(true);
    const fixture = TestBed.createComponent(ToolsPanelComponent);
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const tabs = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="tools-tabs-tab"]'));
    expect(tabs.map((tab) => tab.textContent.trim())).toEqual([
      'Tasks',
      'Changes (1)',
      'Commands (1)',
    ]);
    expect(root.querySelectorAll('[data-testid="task"]')).toHaveLength(1);
    await expectAccessible(root);
    tabs[1]?.click();
    await fixture.whenStable();
    expect(panel.tab()).toBe('changes');
    expect(root.querySelectorAll('[data-testid="file"]')).toHaveLength(1);
    tabs[2]?.click();
    await fixture.whenStable();
    expect(panel.tab()).toBe('commands');
    expect(root.querySelector('[data-testid="command-text"]')?.textContent).toBe('npm run dev');
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="tools-close"]')?.click();
    expect(panel.open()).toBe(false);
  });
});
