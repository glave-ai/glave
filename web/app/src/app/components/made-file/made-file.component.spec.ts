import { LiveAnnouncer } from '@angular/cdk/a11y';
import { signal } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ThreadPanelService } from '../../services/thread-panel.service';
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

/** The chat's panel as the card sees it: the file open in it, and whether it shows. */
interface Panel {
  readonly opened: MadeFile[];
  readonly file: ReturnType<typeof signal<MadeFile | null>>;
  readonly visible: ReturnType<typeof signal<boolean>>;
}

async function render(
  fake: FakeSession,
  file: MadeFile = PACKING,
): Promise<{ readonly fixture: ComponentFixture<MadeFileComponent>; readonly panel: Panel }> {
  const panel: Panel = { opened: [], file: signal(null), visible: signal(false) };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      ThreadService,
      {
        provide: ThreadPanelService,
        useValue: {
          file: panel.file,
          visible: panel.visible,
          openFile: (opened: MadeFile) => panel.opened.push(opened),
        },
      },
    ],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(MadeFileComponent);
  fixture.componentRef.setInput('file', file);
  await settle(fixture);
  return { fixture, panel };
}

function part(root: HTMLElement, name: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="made-file-${name}"]`);
}

describe('MadeFileComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('names the file as the server does, its kind, its parts and where it is kept', async () => {
    const { fixture } = await render(new FakeSession());
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'title')?.textContent).toBe('Packing list');
    expect(part(root, 'line')?.textContent).toBe('Document · 3 pages · Kept with this chat');
    // A plain line stays one line; only a reason wraps.
    expect(part(root, 'line')?.classList.contains('reason')).toBe(false);
    expect(part(root, 'open')?.getAttribute('aria-label')).toBe('Open Packing list');
    expect(part(root, 'download')?.getAttribute('aria-label')).toBe('Download Packing list');
    await expectAccessible(root);
  });

  it('opens the file in the chat’s panel, and is marked while it is open there', async () => {
    const { fixture, panel } = await render(new FakeSession());
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'open')?.click();
    expect(panel.opened).toEqual([PACKING]);
    panel.file.set(PACKING);
    panel.visible.set(true);
    await settle(fixture);
    expect(root.querySelector('.file')?.classList.contains('selected')).toBe(true);
  });

  it('opens a PDF in a tab of the browser’s own, never the panel', async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    const { fixture, panel } = await render(fake, {
      ...PACKING,
      kind: 'pdf',
      file_name: 'plan.pdf',
      media_type: 'application/pdf',
    });
    const tab = { location: { href: '' }, close: vi.fn() };
    const open = vi.spyOn(window, 'open').mockReturnValue(tab as unknown as Window);
    part(fixture.nativeElement as HTMLElement, 'open')?.click();
    await settle(fixture);
    expect(open).toHaveBeenCalledWith('', '_blank');
    expect(tab.location.href).toBe(urls.made[0]);
    expect(panel.opened).toEqual([]);
  });

  it('only downloads a file the server doesn’t preview, and does neither for one not kept', async () => {
    const { fixture } = await render(new FakeSession(), { ...PACKING, previews: false });
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'open')).toBeNull();
    expect(part(root, 'download')).not.toBeNull();
    fixture.componentRef.setInput('file', {
      ...PACKING,
      sha256: null,
      kept: 'Not kept: larger than 10 MB, so it stays on the computer',
    });
    await settle(fixture);
    expect(part(root, 'open')).toBeNull();
    expect(part(root, 'download')).toBeNull();
    expect(part(root, 'line')?.textContent).toContain('Not kept: larger than 10 MB');
    expect(part(root, 'line')?.classList.contains('reason')).toBe(true);
  });

  it('says why a file only downloads where the server couldn’t read it, on lines of its own', async () => {
    const unread = 'glave couldn’t read this file to show it: download it to open it';
    const { fixture } = await render(new FakeSession(), {
      ...PACKING,
      parts: null,
      previews: false,
      unread,
    });
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'line')?.textContent).toBe(`Document · 8 kB · ${unread}`);
    expect(part(root, 'line')?.classList.contains('reason')).toBe(true);
    expect(part(root, 'download')).not.toBeNull();
  });

  it('saves the real file under its name, read back through the server', async () => {
    objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    const { fixture } = await render(fake);
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
    const { fixture } = await render(new FakeSession().answer('file/read', () => gone));
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
    const { fixture } = await render(
      new FakeSession().answer('file/read', () => Promise.reject(fetching)),
    );
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'download')?.click();
    await settle(fixture);
    expect(part(root, 'line')?.textContent).toBe('glave could not be reached. Try again.');
  });
});
