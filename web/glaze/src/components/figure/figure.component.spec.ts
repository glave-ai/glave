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

describe('GlzFigureComponent', () => {
  it('shows the image, named for those who cannot see it, and a link that saves it', async () => {
    const root = await render({ src: PIXEL });
    const image = root.querySelector<HTMLImageElement>('[data-testid="thread-image-image"]');
    expect(image?.getAttribute('src')).toBe(PIXEL);
    expect(image?.getAttribute('alt')).toBe('A red fox at dawn');
    const download = root.querySelector<HTMLAnchorElement>('[data-testid="thread-image-download"]');
    expect(download?.getAttribute('href')).toBe(PIXEL);
    expect(download?.getAttribute('download')).toBe('fox.png');
    expect(download?.textContent).toContain('Download');
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
    expect(failed.textContent).toContain('This file is no longer here');
    expect(failed.querySelector('[data-testid="thread-image-download"]')).toBeNull();
    await expectAccessible(failed);
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
