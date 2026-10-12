import { LiveAnnouncer } from '@angular/cdk/a11y';
import { Clipboard } from '@angular/cdk/clipboard';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { CopyButtonComponent } from './copy-button.component';

async function render(copies: boolean): Promise<{
  readonly root: HTMLElement;
  readonly copy: ReturnType<typeof vi.fn>;
  readonly announce: ReturnType<typeof vi.fn>;
  readonly settle: () => Promise<void>;
}> {
  const copy = vi.fn(() => copies);
  const announce = vi.fn(() => Promise.resolve());
  TestBed.configureTestingModule({
    providers: [
      { provide: Clipboard, useValue: { copy } },
      { provide: LiveAnnouncer, useValue: { announce } },
    ],
  });
  const fixture = TestBed.createComponent(CopyButtonComponent);
  fixture.componentRef.setInput('text', 'They pass now.');
  await fixture.whenStable();
  return {
    root: fixture.nativeElement as HTMLElement,
    copy,
    announce,
    settle: () => fixture.whenStable(),
  };
}

describe('CopyButtonComponent', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('copies the text, says Copied for a moment and announces it', async () => {
    const { root, copy, announce } = await render(true);
    const button = root.querySelector<HTMLButtonElement>('[data-testid="message-copy"]');
    expect(button?.getAttribute('aria-label')).toBe('Copy');
    await expectAccessible(root);
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    button?.click();
    TestBed.tick();
    expect(copy).toHaveBeenCalledWith('They pass now.');
    expect(announce).toHaveBeenCalledWith('Copied');
    expect(button?.getAttribute('aria-label')).toBe('Copied');
    vi.advanceTimersByTime(2000);
    TestBed.tick();
    expect(button?.getAttribute('aria-label')).toBe('Copy');
  });

  it('says so when the clipboard refuses', async () => {
    const { root, announce, settle } = await render(false);
    const button = root.querySelector<HTMLButtonElement>('[data-testid="message-copy"]');
    button?.click();
    await settle();
    expect(announce).toHaveBeenCalledWith('Could not copy');
    expect(button?.getAttribute('aria-label')).toBe('Copy');
  });
});
