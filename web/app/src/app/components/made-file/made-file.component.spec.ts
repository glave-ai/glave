import { LiveAnnouncer } from '@angular/cdk/a11y';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { objectUrls } from '../../testing/object-urls';
import { MadeFileComponent } from './made-file.component';

const PACKING: MadeFile = {
  kind: 'document',
  name: 'Packing list',
  file_name: 'Packing list.docx',
  media_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  bytes: 8_192,
  sha256: 'd'.repeat(64),
  parts: '3 pages',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a document: Packing list (3 pages)',
};

/** A file of three bytes, as `file/read` answers it whole. */
const PIECE = { piece: { media_type: PACKING.media_type, bytes: 3, offset: 0, data: 'UE5H' } };

async function render(
  fake: FakeSession,
  file: MadeFile = PACKING,
): Promise<ComponentFixture<MadeFileComponent>> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(MadeFileComponent);
  fixture.componentRef.setInput('file', file);
  await settle(fixture);
  return fixture;
}

function part(root: HTMLElement, name: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="made-file-${name}"]`);
}

describe('MadeFileComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('names the file as the server does, its kind, its parts and where it is kept', async () => {
    const fixture = await render(new FakeSession());
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'title')?.textContent).toBe('Packing list');
    expect(part(root, 'line')?.textContent).toBe('Document · 3 pages · Kept with this chat');
    // A plain line stays one line; only a reason wraps.
    expect(part(root, 'line')?.classList.contains('reason')).toBe(false);
    expect(part(root, 'download')?.getAttribute('aria-label')).toBe('Download Packing list');
    await expectAccessible(root);
  });

  it('says why a file only downloads, and has nothing to save for one not kept', async () => {
    const unread = 'glave couldn’t read this file to show it: download it to open it';
    const fixture = await render(new FakeSession(), { ...PACKING, parts: null, unread });
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'line')?.textContent).toBe(`Document · 8 kB · ${unread}`);
    expect(part(root, 'line')?.classList.contains('reason')).toBe(true);
    expect(part(root, 'download')).not.toBeNull();
    fixture.componentRef.setInput('file', {
      ...PACKING,
      sha256: null,
      kept: 'Not kept: larger than 10 MB, so it stays on the computer',
    });
    await settle(fixture);
    expect(part(root, 'download')).toBeNull();
    expect(part(root, 'line')?.textContent).toContain('Not kept: larger than 10 MB');
    expect(part(root, 'line')?.classList.contains('reason')).toBe(true);
  });

  it('saves the real file under its name, read back through the server', async () => {
    objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    const fixture = await render(fake);
    const clicked: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(this.download);
    });
    const download = part(fixture.nativeElement as HTMLElement, 'download');
    download?.click();
    fixture.detectChanges();
    // Busy while it reads, and a second click meanwhile saves nothing more.
    expect(download?.getAttribute('aria-busy')).toBe('true');
    download?.click();
    await settle(fixture);
    expect(fake.paramsOf('file/read')).toEqual([{ thread: 'thread-1', sha256: PACKING.sha256 }]);
    expect(clicked).toEqual(['Packing list.docx']);
    expect(download?.getAttribute('aria-busy')).toBeNull();
  });

  it('says why in its line, and aloud, when the file can’t be saved', async () => {
    objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const fixture = await render(new FakeSession().answer('file/read', () => gone));
    const said = vi.spyOn(TestBed.inject(LiveAnnouncer), 'announce').mockResolvedValue();
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'download')?.click();
    await settle(fixture);
    expect(part(root, 'line')?.textContent).toBe('This file is no longer here.');
    expect(said).toHaveBeenCalledWith('This file is no longer here.');
  });

  it('says only that glave wasn’t reached when the browser’s own read fails', async () => {
    objectUrls();
    const fetching = new TypeError('Failed to fetch');
    const fixture = await render(
      new FakeSession().answer('file/read', () => Promise.reject(fetching)),
    );
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'download')?.click();
    await settle(fixture);
    expect(part(root, 'line')?.textContent).toBe('glave could not be reached. Try again.');
  });
});
