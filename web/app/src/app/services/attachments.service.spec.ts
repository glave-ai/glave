import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type FileUploadParams } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { AttachmentsService } from './attachments.service';

const SHA = 'a'.repeat(64);

/** Object URLs the tests can see made and let go of. */
function watchUrls(): { readonly made: string[]; readonly revoked: string[] } {
  const made: string[] = [];
  const revoked: string[] = [];
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: () => {
      const url = `blob:test/${String(made.length)}`;
      made.push(url);
      return url;
    },
  });
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    value: (url: string) => revoked.push(url),
  });
  return { made, revoked };
}

function setUp(fake: FakeSession): AttachmentsService {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), AttachmentsService],
  });
  return TestBed.inject(AttachmentsService);
}

/** Until every file has an answer. */
async function settled(attachments: AttachmentsService): Promise<void> {
  await vi.waitFor(() => {
    expect(attachments.waiting()).toBe(false);
  });
}

describe('AttachmentsService', () => {
  it('keeps each file added in the project, showing an image as itself', async () => {
    const urls = watchUrls();
    const fake = new FakeSession().answer('file/upload', (params: FileUploadParams) => ({
      file: { sha256: SHA, name: params.name, media_type: params.media_type, bytes: 3 },
    }));
    const attachments = setUp(fake);
    attachments.keepIn('p1');
    attachments.add([new File(['png'], 'cat.png', { type: 'image/png' })]);
    expect(attachments.waiting()).toBe(true);
    expect(attachments.detailOf(attachments.attachments()[0]!)).toBe('Adding');
    await settled(attachments);
    expect(fake.paramsOf('file/upload')).toEqual([
      { project: 'p1', name: 'cat.png', media_type: 'image/png', data: 'cG5n' },
    ]);
    const [cat] = attachments.attachments();
    expect(cat?.preview).toBe(urls.made[0]);
    expect(attachments.detailOf(cat!)).toBe('3 B');
    expect(attachments.attached()).toEqual([{ sha256: SHA, name: 'cat.png' }]);
    attachments.remove(cat!.key);
    expect(urls.revoked).toEqual(urls.made);
    expect(attachments.attachments()).toEqual([]);
  });

  it("shows the server's reason for a file it refuses, and refuses one too large to send", async () => {
    watchUrls();
    const refusal = new ApiError({
      code: ErrorCode.INVALID_PARAMS,
      message: "glave can't use videos yet: add a screenshot or a few images instead",
    });
    const attachments = setUp(new FakeSession().answer('file/upload', refusal));
    const huge = new File([new Uint8Array(10 * 1024 * 1024 + 1)], 'huge.bin');
    attachments.add([new File(['mp4'], 'clip.mp4', { type: 'video/mp4' }), huge]);
    await settled(attachments);
    const [clip, big] = attachments.attachments();
    expect(clip?.state).toBe('refused');
    expect(attachments.detailOf(clip!)).toContain("glave can't use videos yet");
    expect(big?.mediaType).toBe('application/octet-stream');
    expect(attachments.detailOf(big!)).toBe('Too large to add: files up to 10 MB');
    expect(attachments.attached()).toEqual([]);
  });

  it('sends the files again when the message is to go to another project', async () => {
    watchUrls();
    const fake = new FakeSession().answer('file/upload', (params: FileUploadParams) => ({
      file: { sha256: SHA, name: params.name, media_type: params.media_type, bytes: 2 },
    }));
    const attachments = setUp(fake);
    attachments.add([new File(['hi'], 'notes.txt', { type: 'text/plain' })]);
    await settled(attachments);
    attachments.keepIn('p2');
    await settled(attachments);
    const sent = fake.paramsOf('file/upload') as readonly FileUploadParams[];
    expect(sent.map((params) => params.project ?? null)).toEqual([null, 'p2']);
    attachments.clear();
    expect(attachments.attachments()).toEqual([]);
  });
});
