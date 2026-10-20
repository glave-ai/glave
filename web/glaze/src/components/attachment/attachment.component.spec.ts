import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzAttachmentComponent } from './attachment.component';

async function render(inputs: Record<string, unknown>): Promise<{
  readonly root: HTMLElement;
  readonly removed: ReturnType<typeof vi.fn>;
}> {
  const fixture = TestBed.createComponent(GlzAttachmentComponent);
  fixture.componentRef.setInput('testId', 'composer-file');
  for (const [name, value] of Object.entries(inputs)) {
    fixture.componentRef.setInput(name, value);
  }
  const removed = vi.fn();
  fixture.componentInstance.remove.subscribe(removed);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, removed };
}

describe('GlzAttachmentComponent', () => {
  it('shows an image, its name and size, and takes it off with its button', async () => {
    const { root, removed } = await render({
      name: 'screenshot.png',
      detail: '214 KB',
      preview: 'data:image/gif;base64,R0lGODlhAQABAAAAACw=',
      removeLabel: 'Remove screenshot.png',
    });
    expect(root.querySelector('img')?.getAttribute('alt')).toBe('');
    expect(root.textContent).toContain('screenshot.png');
    expect(root.textContent).toContain('214 KB');
    const remove = root.querySelector<HTMLButtonElement>('[data-testid="composer-file-remove"]');
    expect(remove?.getAttribute('aria-label')).toBe('Remove screenshot.png');
    remove?.click();
    expect(removed).toHaveBeenCalledOnce();
    await expectAccessible(root);
  });

  it("shows a file's mark, says it is busy, and has no button without a label", async () => {
    const { root } = await render({ name: 'notes.txt', detail: 'Uploading', busy: true });
    expect(root.querySelector('img')).toBeNull();
    expect(root.getAttribute('aria-busy')).toBe('true');
    expect(root.querySelector('button')).toBeNull();
    await expectAccessible(root);
  });

  it('says why it was refused, marked as failed', async () => {
    const { root } = await render({
      name: 'clip.mp4',
      detail: "glave can't use videos yet",
      failed: true,
      removeLabel: 'Remove clip.mp4',
    });
    expect(root.classList).toContain('glz-attachment--failed');
    expect(root.textContent).toContain("glave can't use videos yet");
    expect(root.getAttribute('aria-busy')).toBeNull();
    await expectAccessible(root);
  });
});
