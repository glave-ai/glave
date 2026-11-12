import { TestBed } from '@angular/core/testing';

import { GLZ_TEXT, type GlzText } from '../../tokens/text';
import { expectAccessible } from '../../testing/axe';
import { GlzFigureComponent } from './figure.component';

/** A one-pixel GIF. */
const PIXEL = 'data:image/gif;base64,R0lGODlhAQABAAAAACw=';

async function render(inputs: Record<string, unknown>): Promise<HTMLElement> {
  const fixture = TestBed.createComponent(GlzFigureComponent);
  fixture.componentRef.setInput('testId', 'thread-image');
  fixture.componentRef.setInput('alt', 'A red fox at dawn');
  fixture.componentRef.setInput('name', 'fox.png');
  for (const [name, value] of Object.entries(inputs)) {
    fixture.componentRef.setInput(name, value);
  }
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

/** The figure's two rows, the frame and the link's, as the classes that size them. */
function rows(root: HTMLElement): readonly string[] {
  return Array.from(root.querySelector('figure')?.children ?? [], (row) => row.className);
}

/** The link that saves the image. */
function download(root: HTMLElement): HTMLAnchorElement | null {
  return root.querySelector<HTMLAnchorElement>('[data-testid="thread-image-download"]');
}

describe('GlzFigureComponent', () => {
  it('shows the image, named for those who cannot see it, and a link that saves it', async () => {
    const root = await render({ src: PIXEL });
    const image = root.querySelector<HTMLImageElement>('[data-testid="thread-image-image"]');
    expect(image?.getAttribute('src')).toBe(PIXEL);
    expect(image?.getAttribute('alt')).toBe('A red fox at dawn');
    expect(download(root)?.getAttribute('href')).toBe(PIXEL);
    expect(download(root)?.getAttribute('download')).toBe('fox.png');
    expect(download(root)?.textContent).toContain('Download');
    expect(download(root)?.closest('.unseen')).toBeNull();
    expect(root.getAttribute('aria-busy')).toBeNull();
    await expectAccessible(root);
  });

  it('holds its place while it comes, and says why when it cannot be shown', async () => {
    const coming = await render({});
    expect(coming.getAttribute('aria-busy')).toBe('true');
    expect(coming.querySelector('img')).toBeNull();
    expect(coming.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe(
      'A red fox at dawn',
    );
    await expectAccessible(coming);
    const failed = await render({ failed: 'This file is no longer here' });
    expect(failed.getAttribute('aria-busy')).toBeNull();
    // The reason is read as it is, not hidden behind a name for an image that isn't there.
    const why = failed.querySelector('[data-testid="thread-image-failed"]');
    expect(why?.textContent).toContain('This file is no longer here');
    expect(why?.getAttribute('role')).toBeNull();
    await expectAccessible(failed);
  });

  it('is as tall in every state: the frame, then the link’s row, unseen with nothing to save', async () => {
    const shown = await render({ src: PIXEL });
    const coming = await render({});
    const failed = await render({ failed: 'This file is no longer here' });
    expect(rows(shown)).toEqual(['frame image', '']);
    for (const root of [coming, failed]) {
      expect(rows(root)).toEqual([expect.stringMatching(/^frame\b/u), 'unseen']);
      expect(download(root)?.hasAttribute('href')).toBe(false);
      expect(download(root)?.className).toBe(download(shown)?.className);
    }
  });

  it("writes its word in the app's language", async () => {
    TestBed.configureTestingModule({
      providers: [
        { provide: GLZ_TEXT, useValue: { figure: { downloadLabel: 'Télécharger' } } as GlzText },
      ],
    });
    const root = await render({ src: PIXEL });
    expect(root.textContent).toContain('Télécharger');
  });
});
