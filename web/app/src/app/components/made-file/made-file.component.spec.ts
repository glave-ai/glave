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
  seq: 7,
  sha256: 'b'.repeat(64),
  name: 'Trip budget.xlsx',
  title: 'Trip budget',
  kind: 'spreadsheet',
  media_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  bytes: 12_288,
  kept: 'Kept with this chat',
};

/** A file of three bytes, as `file/read` answers it whole. */
const PIECE = { piece: { media_type: BUDGET.media_type, bytes: 3, offset: 0, data: 'UE5H' } };

async function render(
  fake: FakeSession,
  openFile: (file: MadeFile) => void = () => undefined,
): Promise<ComponentFixture<MadeFileComponent>> {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      ThreadService,
      { provide: ThreadPanelService, useValue: { openFile } },
    ],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(MadeFileComponent);
  fixture.componentRef.setInput('file', BUDGET);
  await settle(fixture);
  return fixture;
}

function part(root: HTMLElement, name: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="made-file-${name}"]`);
}

describe('MadeFileComponent', () => {
  it('names the file by its title, then its kind, size and where the server says it is kept', async () => {
    const root = (await render(new FakeSession())).nativeElement as HTMLElement;
    expect(part(root, 'title')?.textContent).toBe('Trip budget');
    expect(part(root, 'line')?.textContent).toBe('Spreadsheet · 12 kB · Kept with this chat');
    expect(part(root, 'download')?.getAttribute('aria-label')).toBe('Download Trip budget.xlsx');
    await expectAccessible(root);
  });

  it('opens the file in the chat’s panel', async () => {
    const opened: MadeFile[] = [];
    const fixture = await render(new FakeSession(), (file) => opened.push(file));
    part(fixture.nativeElement as HTMLElement, 'open')?.click();
    expect(opened).toEqual([BUDGET]);
  });

  it('saves the real file under its name, read back through the server', async () => {
    objectUrls();
    const fake = new FakeSession().answer('file/read', PIECE);
    const fixture = await render(fake);
    const clicked: HTMLAnchorElement[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(this);
    });
    part(fixture.nativeElement as HTMLElement, 'download')?.click();
    await settle(fixture);
    expect(fake.paramsOf('file/read')).toEqual([{ thread: 'thread-1', sha256: BUDGET.sha256 }]);
    expect(clicked.map((link) => link.download)).toEqual(['Trip budget.xlsx']);
  });

  it('says why in its line when the file can’t be saved', async () => {
    objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const fixture = await render(new FakeSession().answer('file/read', () => gone));
    const root = fixture.nativeElement as HTMLElement;
    part(root, 'download')?.click();
    await settle(fixture);
    expect(part(root, 'line')?.textContent).toBe('This file is no longer here.');
  });
});
