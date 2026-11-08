import { TestBed } from '@angular/core/testing';
import type { Preview } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession } from '../../testing/fake-session';
import { command, event } from '../../testing/fixtures';
import { drawn, openForPanel } from '../../testing/thread-panel';
import { ThreadPreviewComponent } from './thread-preview.component';

const STARTED = [
  event(1, 'command.started', { command: 'c1', script: 'npm run dev' }),
  event(2, 'command.started', { command: 'c2', script: 'npm run docs' }),
];

const DEV: Preview = { command: 'c1', port: 5173, url: 'http://127.0.0.1:61234/' };
const DOCS: Preview = { command: 'c2', port: 3000, url: 'http://127.0.0.1:61235/' };

async function render(previews: readonly Preview[]): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadPreviewComponent>>;
}> {
  const fake = new FakeSession().answer('command/list', {
    commands: [command('c1', 'npm run dev'), command('c2', 'npm run docs')],
    previews,
  });
  await openForPanel(fake, STARTED);
  const fixture = TestBed.createComponent(ThreadPreviewComponent);
  await drawn(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function one(root: HTMLElement, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

describe('ThreadPreviewComponent', () => {
  it('frames the preview, sandboxed at its own origin, with its address, Reload and a new tab', async () => {
    const { root, fixture } = await render([DEV]);
    expect(one(root, 'preview-address')?.textContent).toBe(DEV.url);
    const frame = one(root, 'preview-frame') as HTMLIFrameElement | null;
    expect(frame?.getAttribute('src')).toBe(DEV.url);
    expect(frame?.getAttribute('sandbox')).toBe(
      'allow-scripts allow-same-origin allow-forms allow-popups',
    );
    expect(frame?.getAttribute('referrerpolicy')).toBe('no-referrer');
    expect(frame?.title).toBe('What npm run dev serves');
    const open = one(root, 'preview-open') as HTMLAnchorElement | null;
    expect(open?.textContent.trim()).toBe('Open in a new tab');
    expect([open?.href, open?.target, open?.rel]).toEqual([
      DEV.url,
      '_blank',
      'noopener noreferrer',
    ]);
    // One preview: nothing to choose.
    expect(one(root, 'preview-which')).toBeNull();
    expect(root.textContent).toContain('Some apps refuse to be shown inside another page.');
    // Axe can't look inside a frame here; the end-to-end tests check the page with it.
    await expectAccessible(one(root, 'preview-bar') ?? root);

    one(root, 'preview-reload')?.click();
    await fixture.whenStable();
    const again = one(root, 'preview-frame');
    expect(again).not.toBe(frame);
    expect(again?.getAttribute('src')).toBe(DEV.url);
  });

  it('chooses among several, the first until the person chooses', async () => {
    const { root, fixture } = await render([DEV, DOCS]);
    expect(one(root, 'preview-address')?.textContent).toBe(DEV.url);
    const which = one(root, 'preview-which-trigger');
    expect(which?.textContent).toContain('npm run dev, port 5173');
    await expectAccessible(one(root, 'preview-bar') ?? root);
    which?.click();
    await drawn(fixture);
    const options = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"]'));
    expect(options.map((each) => each.textContent.trim())).toEqual([
      'npm run dev, port 5173',
      'npm run docs, port 3000',
    ]);
    options[1]?.click();
    await drawn(fixture);
    expect(one(root, 'preview-address')?.textContent).toBe(DOCS.url);
    expect(one(root, 'preview-frame')?.getAttribute('src')).toBe(DOCS.url);
  });

  it('frames nothing but a page on this computer’s loopback, and still links to it', async () => {
    const elsewhere: Preview = { ...DEV, url: 'https://example.com/' };
    const { root } = await render([elsewhere]);
    expect(one(root, 'preview-frame')).toBeNull();
    expect((one(root, 'preview-open') as HTMLAnchorElement | null)?.href).toBe(elsewhere.url);
  });

  it('shows nothing while nothing serves', async () => {
    const { root } = await render([]);
    expect(root.textContent.trim()).toBe('');
  });
});
