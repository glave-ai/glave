import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { ToolsPanelService } from '../../services/tools-panel.service';
import { ThreadHeaderComponent } from './thread-header.component';

async function render(narrow: boolean): Promise<{
  readonly root: HTMLElement;
  readonly renamed: () => number;
}> {
  localStorage.clear();
  TestBed.configureTestingModule({
    providers: [
      { provide: BreakpointObserver, useValue: { observe: () => of({ matches: narrow }) } },
    ],
  });
  const fixture = TestBed.createComponent(ThreadHeaderComponent);
  fixture.componentRef.setInput('title', 'Tidy the repo');
  fixture.componentRef.setInput('space', 'glave');
  fixture.componentRef.setInput('mark', { text: 'needs you', tone: 'accent', variant: 'dot' });
  let renames = 0;
  fixture.componentInstance.rename.subscribe(() => (renames += 1));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, renamed: () => renames };
}

describe('ThreadHeaderComponent', () => {
  it('shows the space, the title and its mark, and asks to rename', async () => {
    const { root, renamed } = await render(false);
    expect(root.querySelector('[data-testid="thread-space"]')?.textContent).toBe('glave');
    expect(root.querySelector('h1[data-testid="thread-title"]')?.textContent).toBe('Tidy the repo');
    expect(root.querySelector('glz-badge')?.textContent.trim()).toBe('needs you');
    expect(root.querySelector('[data-testid="thread-tools"]')).toBeNull();
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="thread-rename"]')?.click();
    expect(renamed()).toBe(1);
  });

  it('opens the tools sheet on narrow screens', async () => {
    const { root } = await render(true);
    const tools = root.querySelector<HTMLButtonElement>('[data-testid="thread-tools"]');
    expect(tools?.getAttribute('aria-pressed')).toBe('false');
    await expectAccessible(root);
    tools?.click();
    expect(TestBed.inject(ToolsPanelService).open()).toBe(true);
  });
});
