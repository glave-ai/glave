import { DestroyRef, LOCALE_ID, Service, computed, inject, signal } from '@angular/core';

import {
  type Attachment,
  MAX_SENT,
  attachedOf,
  base64Of,
  mediaTypeOf,
  shown,
  sizeOf,
} from '../lib/attachments';
import { SessionService } from './session.service';

/** A file as its chip shows it. */
export interface Chip {
  readonly key: string;
  readonly name: string;
  /** Its size, that it is on its way, or why it was refused. */
  readonly detail: string;
  readonly preview: string | null;
  readonly busy: boolean;
  readonly failed: boolean;
  /** Names the button that takes it off the message. */
  readonly removeLabel: string;
}

/** One file added, with the file itself until it is kept, so it can be sent again elsewhere. */
interface Held {
  readonly attachment: Attachment;
  readonly file: File;
}

/**
 * The files added to the message being written: each sent to be kept in the project the
 * message goes to (`file/upload`) as soon as it is added, shown in a chip meanwhile (the image
 * itself, for an image), and named by the message once kept. A file the server refuses stays
 * in its chip with the server's reason until it is removed. When the message is to go to
 * another project, the files are sent there again. Each composer provides its own.
 */
@Service({ autoProvided: false })
export class AttachmentsService {
  private readonly session = inject(SessionService);
  private readonly locale = inject(LOCALE_ID);
  private readonly held = signal<readonly Held[]>([]);
  /** The project files are kept in; `null` for the person's Chats. */
  private project: string | null = null;
  private added = 0;

  /** The files added, in order. */
  readonly attachments = computed(() => this.held().map(({ attachment }) => attachment));
  /** Each file as its chip shows it. */
  readonly chips = computed<readonly Chip[]>(() =>
    this.attachments().map((attachment) => ({
      key: attachment.key,
      name: attachment.name,
      detail: this.detailOf(attachment),
      preview: attachment.preview,
      busy: attachment.state === 'sending',
      failed: attachment.state === 'refused',
      removeLabel: $localize`:@@attachments.remove:Remove ${attachment.name}:FILE:`,
    })),
  );
  /** How many files are kept, ready to go with the message. */
  readonly count = computed(() => this.attached().length);
  /** The files kept, as `turn/start` names them. */
  readonly attached = computed(() => attachedOf(this.attachments()));
  /** Whether a file is still on its way: the message waits for it. */
  readonly waiting = computed(() =>
    this.attachments().some((attachment) => attachment.state === 'sending'),
  );

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      this.clear();
    });
  }

  /** Sends each of `files` to be kept, with a chip for each meanwhile. */
  add(files: readonly File[]): void {
    for (const file of files) {
      this.added += 1;
      const mediaType = mediaTypeOf(file);
      const attachment: Attachment = {
        key: `file-${String(this.added)}`,
        name: file.name,
        mediaType,
        bytes: file.size,
        preview: shown(mediaType) ? URL.createObjectURL(file) : null,
        state: 'sending',
        sha256: null,
        reason: null,
      };
      this.held.update((held) => [...held, { attachment, file }]);
      void this.keep(attachment.key, file, this.project);
    }
  }

  /** Takes the file `key` off the message. */
  remove(key: string): void {
    const gone = this.held().find(({ attachment }) => attachment.key === key);
    this.revoke(gone);
    this.held.update((held) => held.filter(({ attachment }) => attachment.key !== key));
  }

  /** Takes every file off, once the message is sent. */
  clear(): void {
    for (const held of this.held()) {
      this.revoke(held);
    }
    this.held.set([]);
  }

  /** Keeps files in `project` from now on, sending again what was kept elsewhere. */
  keepIn(project: string | null): void {
    if (project === this.project) {
      return;
    }
    this.project = project;
    for (const { attachment, file } of this.held()) {
      this.set(attachment.key, { state: 'sending', sha256: null, reason: null });
      void this.keep(attachment.key, file, project);
    }
  }

  /** What a chip says under the file's name: its size, that it is on its way, or the reason. */
  detailOf(attachment: Attachment): string {
    switch (attachment.state) {
      case 'sending':
        return $localize`:@@attachments.sending:Adding`;
      case 'refused':
        return attachment.reason ?? '';
      case 'kept':
        return sizeOf(attachment.bytes, this.locale);
    }
  }

  private async keep(key: string, file: File, project: string | null): Promise<void> {
    if (file.size > MAX_SENT) {
      const most = sizeOf(MAX_SENT, this.locale);
      this.set(key, {
        state: 'refused',
        reason: $localize`:@@attachments.tooLarge:Too large to add: files up to ${most}:SIZE:`,
      });
      return;
    }
    try {
      const data = base64Of(new Uint8Array(await file.arrayBuffer()));
      const { file: kept } = await this.session.call('file/upload', {
        ...(project === null ? {} : { project }),
        name: file.name,
        media_type: mediaTypeOf(file),
        data,
      });
      // Sent again elsewhere meanwhile: that one answers.
      if (project === this.project) {
        this.set(key, { state: 'kept', sha256: kept.sha256, name: kept.name });
      }
    } catch (error: unknown) {
      if (project === this.project) {
        const reason = error instanceof Error ? error.message : String(error);
        this.set(key, { state: 'refused', reason });
      }
    }
  }

  private set(key: string, change: Partial<Attachment>): void {
    this.held.update((held) =>
      held.map((one) =>
        one.attachment.key === key ? { ...one, attachment: { ...one.attachment, ...change } } : one,
      ),
    );
  }

  private revoke(held: Held | undefined): void {
    if (held?.attachment.preview !== null && held?.attachment.preview !== undefined) {
      URL.revokeObjectURL(held.attachment.preview);
    }
  }
}
