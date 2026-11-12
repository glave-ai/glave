import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import type { HeldImage } from '../../lib/conversation';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { objectUrls } from '../../testing/object-urls';
import { ThreadImageComponent } from './thread-image.component';

const FOX: HeldImage = { sha256: 'f'.repeat(64), mediaType: 'image/png', name: 'image.png' };

async function render(fake: FakeSession): Promise<HTMLElement> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadImageComponent);
  fixture.componentRef.setInput('image', FOX);
  fixture.componentRef.setInput('alt', 'image_generate: a red fox at dawn');
  await settle(fixture);
  return fixture.nativeElement as HTMLElement;
}

describe('ThreadImageComponent', () => {
  it('draws the image the thread holds, read back through the server, to save', async () => {
    const urls = objectUrls();
    const piece = { media_type: 'image/png', bytes: 3, offset: 0, data: 'UE5H' };
    const fake = new FakeSession().answer('file/read', { piece });
    const root = await render(fake);
    expect(fake.paramsOf('file/read')).toEqual([{ thread: 'thread-1', sha256: FOX.sha256 }]);
    const image = root.querySelector('[data-testid="thread-image-image"]');
    expect(image?.getAttribute('src')).toBe(urls.made[0]);
    expect(image?.getAttribute('alt')).toBe('image_generate: a red fox at dawn');
    const download = root.querySelector('[data-testid="thread-image-download"]');
    expect(download?.getAttribute('download')).toBe('image.png');
    await expectAccessible(root);
  });

  it("says why when the server can't give it", async () => {
    objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const root = await render(new FakeSession().answer('file/read', () => gone));
    expect(root.textContent).toContain('this file is no longer here');
    expect(root.querySelector('img')).toBeNull();
    await expectAccessible(root);
  });
});
