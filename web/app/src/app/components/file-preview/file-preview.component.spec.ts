import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { objectUrls } from '../../testing/object-urls';
import { FilePreviewComponent } from './file-preview.component';

const BUDGET: MadeFile = {
  seq: 7,
  sha256: 'b'.repeat(64),
  name: 'budget.csv',
  title: 'budget',
  kind: 'spreadsheet',
  media_type: 'text/csv',
  bytes: 23,
  kept: 'Kept with this chat',
};

const PLAN: MadeFile = {
  ...BUDGET,
  sha256: 'p'.repeat(64),
  name: 'plan.pdf',
  title: 'plan',
  kind: 'pdf',
  media_type: 'application/pdf',
};

async function render(
  fake: FakeSession,
  file: MadeFile,
): Promise<ComponentFixture<FilePreviewComponent>> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(FilePreviewComponent);
  fixture.componentRef.setInput('file', file);
  await settle(fixture);
  return fixture;
}

function part(root: HTMLElement, name: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="file-preview-${name}"]`);
}

describe('FilePreviewComponent', () => {
  it('shows a spreadsheet as the server previews it: a table', async () => {
    const markdown = '| Item | Total |\n|---|---|\n| Flights | 780 |';
    const fake = new FakeSession().answer('file/preview', { markdown });
    const root = (await render(fake, BUDGET)).nativeElement as HTMLElement;
    expect(fake.paramsOf('file/preview')).toEqual([{ thread: 'thread-1', sha256: BUDGET.sha256 }]);
    expect(part(root, 'title')?.textContent).toBe('budget');
    expect(root.querySelector('table')?.textContent).toContain('Flights');
    await expectAccessible(root);
  });

  it('draws a PDF in the browser, from the bytes read back through the server', async () => {
    const urls = objectUrls();
    const piece = { piece: { media_type: 'application/pdf', bytes: 3, offset: 0, data: 'UE5H' } };
    const fake = new FakeSession().answer('file/read', piece);
    const root = (await render(fake, PLAN)).nativeElement as HTMLElement;
    expect(fake.paramsOf('file/preview')).toEqual([]);
    const frame = part(root, 'frame');
    expect(frame?.getAttribute('src')).toBe(urls.made[0]);
    expect(frame?.getAttribute('title')).toBe('plan');
  });

  it('says why in the server’s words when it can’t be shown, beside Download', async () => {
    const refused = new ApiError({
      code: ErrorCode.INVALID_PARAMS,
      message: "this file can't be shown here: download it to open it",
    });
    const fake = new FakeSession().answer('file/preview', () => refused);
    const root = (await render(fake, BUDGET)).nativeElement as HTMLElement;
    expect(part(root, 'failed')?.textContent).toBe(
      "This file can't be shown here: download it to open it.",
    );
    expect(part(root, 'download')).not.toBeNull();
    await expectAccessible(root);
  });
});
