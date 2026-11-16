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

const BUDGET: MadeFile = {
  kind: 'spreadsheet',
  name: 'Trip budget',
  file_name: 'Trip budget.xlsx',
  media_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  bytes: 12_288,
  sha256: 'b'.repeat(64),
  parts: '2 sheets',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a spreadsheet: Trip budget (2 sheets)',
};

/** A file of three bytes, as `file/read` answers it whole. */
const PIECE = { piece: { media_type: BUDGET.media_type, bytes: 3, offset: 0, data: 'UE5H' } };

/** The chat's panel as the card sees it: the file open in it, and whether it shows. */
interface Panel {
  readonly opened: MadeFile[];
  readonly file: ReturnType<typeof signal<MadeFile | null>>;
  readonly visible: ReturnType<typeof signal<boolean>>;
}

async function render(
  fake: FakeSession,
  file: MadeFile = BUDGET,
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
    expect(part(root, 'title')?.textContent).toBe('Trip budget');
    expect(part(root, 'line')?.textContent).toBe('Spreadsheet · 2 sheets · Kept with this chat');
    expect(part(root, 'open')?.getAttribute('aria-label')).toBe('Open Trip budget');
    expect(part(root, 'download')?.getAttribute('aria-label')).toBe('Download Trip budget');
    await expectAccessible(root);
  });

  it('opens the file in the chat’s panel, and is marked while it is open there', async () => {
    const { fixture, panel } = await render(new FakeSession());
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'open')?.click();
    expect(panel.opened).toEqual([BUDGET]);
    panel.file.set(BUDGET);
    panel.visible.set(true);
    await settle(fixture);
    expect(root.querySelector('.file')?.classList.contains('selected')).toBe(true);
  });

  it('opens a PDF in a tab of the browser’s own, never the panel', async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    const { fixture, panel } = await render(fake, {
      ...BUDGET,
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
    const { fixture } = await render(new FakeSession(), { ...BUDGET, previews: false });
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'open')).toBeNull();
    expect(part(root, 'download')).not.toBeNull();
    fixture.componentRef.setInput('file', {
      ...BUDGET,
      sha256: null,
      kept: 'Not kept: larger than 10 MB, so it stays on the computer',
    });
    await settle(fixture);
    expect(part(root, 'open')).toBeNull();
    expect(part(root, 'download')).toBeNull();
    expect(part(root, 'line')?.textContent).toContain('Not kept: larger than 10 MB');
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
    part(fixture.nativeElement as HTMLElement, 'download')?.click();
    await settle(fixture);
    expect(fake.paramsOf('file/read')).toEqual([{ thread: 'thread-1', sha256: BUDGET.sha256 }]);
    expect(clicked).toEqual(['Trip budget.xlsx']);
  });

  it('says why in its line when the file can’t be saved', async () => {
    objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const { fixture } = await render(new FakeSession().answer('file/read', () => gone));
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'download')?.click();
    await settle(fixture);
    expect(part(root, 'line')?.textContent).toBe('This file is no longer here.');
  });
});
