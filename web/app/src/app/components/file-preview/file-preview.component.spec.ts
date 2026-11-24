import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type MadeFile } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ServerService } from '../../services/server.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { FilePreviewComponent } from './file-preview.component';

const BUDGET: MadeFile = {
  kind: 'spreadsheet',
  name: 'Trip budget',
  file_name: 'budget.csv',
  media_type: 'text/csv',
  bytes: 23,
  sha256: 'b'.repeat(64),
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a spreadsheet: Trip budget',
};

async function render(
  fake: FakeSession,
  file: MadeFile,
): Promise<ComponentFixture<FilePreviewComponent>> {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake), ThreadService],
  });
  await TestBed.inject(ServerService).load();
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
  it('shows a spreadsheet as the server previews it, and what it left out', async () => {
    const markdown = '| Item | Total |\n|---|---|\n| Flights | 780 |';
    const cut = 'This shows the start of the file: download it to see the rest.';
    const fake = new FakeSession().answer('file/preview', { preview: { markdown, cut } });
    const root = (await render(fake, BUDGET)).nativeElement as HTMLElement;
    expect(fake.paramsOf('file/preview')).toEqual([{ thread: 'thread-1', sha256: BUDGET.sha256 }]);
    expect(part(root, 'title')?.textContent).toBe('Trip budget');
    expect(root.querySelector('table')?.textContent).toContain('Flights');
    expect(part(root, 'cut')?.textContent).toBe(cut);
    await expectAccessible(root);
  });

  it('shares a link where the server shares files, never the file itself', async () => {
    Object.defineProperty(navigator, 'canShare', { configurable: true, value: () => true });
    const fake = new FakeSession()
      .answer('server/discover', { ...SERVER, shares: ['anyone'] })
      .answer('share/get', {});
    const fixture = await render(fake, BUDGET);
    const root = fixture.nativeElement as HTMLElement;
    expect(part(root, 'share')).toBeNull();
    part(root, 'share-link')?.click();
    await settle(fixture);
    expect(fake.paramsOf('share/get')).toEqual([
      { kind: 'file', thread: 'thread-1', sha256: BUDGET.sha256 },
    ]);
    Reflect.deleteProperty(navigator, 'canShare');
  });

  it('reads the file as it opens where the browser shares files, so the tap shares it at once', async () => {
    const shared: ShareData[] = [];
    Object.defineProperty(navigator, 'canShare', { configurable: true, value: () => true });
    Object.defineProperty(navigator, 'share', {
      configurable: true,
      value: (data: ShareData) => {
        shared.push(data);
        return Promise.resolve();
      },
    });
    try {
      const piece = { piece: { media_type: 'text/csv', bytes: 3, offset: 0, data: 'YSxi' } };
      const fake = new FakeSession()
        .answer('file/preview', { preview: { markdown: 'a,b' } })
        .answer('file/read', piece);
      const fixture = await render(fake, BUDGET);
      const root = fixture.nativeElement as HTMLElement;
      part(root, 'share')?.click();
      await settle(fixture);
      expect(shared.map((data) => data.files?.[0]?.name)).toEqual(['budget.csv']);
    } finally {
      Reflect.deleteProperty(navigator, 'canShare');
      Reflect.deleteProperty(navigator, 'share');
    }
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
