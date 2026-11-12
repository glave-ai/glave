import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import type { HeldImage } from '../../lib/conversation';
import { FilesService } from '../../services/files.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { objectUrls } from '../../testing/object-urls';
import { ThreadImageComponent } from './thread-image.component';

const FOX: HeldImage = { sha256: 'f'.repeat(64), mediaType: 'image/png', name: 'fox.png' };

/** A file of three bytes, as `file/read` answers it whole. */
const PIECE = { piece: { media_type: 'image/png', bytes: 3, offset: 0, data: 'UE5H' } };

function setUp(fake: FakeSession): void {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
}

/** The image drawn in `thread-1`. */
async function draw(image: HeldImage = FOX): Promise<ComponentFixture<ThreadImageComponent>> {
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadImageComponent);
  fixture.componentRef.setInput('image', image);
  await settle(fixture);
  return fixture;
}

describe('ThreadImageComponent', () => {
  it('draws the image the thread holds, read back through the server, to save', async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    setUp(fake);
    const root = (await draw()).nativeElement as HTMLElement;
    expect(fake.paramsOf('file/read')).toEqual([{ thread: 'thread-1', sha256: FOX.sha256 }]);
    const image = root.querySelector('[data-testid="thread-image-image"]');
    expect(image?.getAttribute('src')).toBe(urls.made[0]);
    // Named as it is saved: the step's line, `read fox.png`, says what was done, not what it shows.
    expect(image?.getAttribute('alt')).toBe('fox.png');
    const download = root.querySelector('[data-testid="thread-image-download"]');
    expect(download?.getAttribute('download')).toBe('fox.png');
    await expectAccessible(root);
  });

  it("says why when the server can't give it", async () => {
    objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    setUp(new FakeSession().answer('file/read', () => gone));
    const root = (await draw()).nativeElement as HTMLElement;
    expect(root.textContent).toContain('this file is no longer here');
    expect(root.querySelector('img')).toBeNull();
    await expectAccessible(root);
  });

  it('holds the image while it is drawn, and lets go of it once it no longer is', async () => {
    objectUrls();
    setUp(new FakeSession().answer('file/read', PIECE));
    const files = TestBed.inject(FilesService);
    const hold = files.hold.bind(files);
    const released: string[] = [];
    vi.spyOn(files, 'hold').mockImplementation((thread, sha256, mediaType) => {
      const held = hold(thread, sha256, mediaType);
      return {
        url: held.url,
        release: () => {
          released.push(sha256);
          held.release();
        },
      };
    });
    const fixture = await draw();
    const other = { ...FOX, sha256: 'e'.repeat(64) };
    fixture.componentRef.setInput('image', other);
    await settle(fixture);
    expect(released).toEqual([FOX.sha256]);
    fixture.destroy();
    expect(released).toEqual([FOX.sha256, other.sha256]);
  });
});
