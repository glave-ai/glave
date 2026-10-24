import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import type { Remembered } from '@glave/client';
import {
  GlzBadgeComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzCheckboxComponent,
  GlzConfirmDialogComponent,
  GlzDialogComponent,
  GlzEmptyStateComponent,
  GlzErrorComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';
import { Brain } from 'lucide';

import { MemoryService } from '../../services/memory.service';
import { PageHeaderComponent } from '../page-header/page-header.component';

/** The memory page: each memory, with Edit and Forget. */
@Component({
  selector: 'app-memory',
  imports: [
    PageHeaderComponent,
    ReactiveFormsModule,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzCheckboxComponent,
    GlzConfirmDialogComponent,
    GlzDialogComponent,
    GlzEmptyStateComponent,
    GlzErrorComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
  ],
  templateUrl: './memory.component.html',
  styleUrl: './memory.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MemoryComponent {
  protected readonly copy = {
    heading: $localize`:@@memory.heading:Memory`,
    intro: $localize`:@@memory.intro:What glave keeps between threads: about you, and about this project. Edit or forget any of it.`,
    personal: $localize`:@@memory.personal:about you`,
    project: $localize`:@@memory.project:this project`,
    edit: $localize`:@@memory.edit:Edit`,
    forget: $localize`:@@memory.forget:Forget`,
    editHeading: $localize`:@@memory.editHeading:Edit the memory`,
    text: $localize`:@@memory.text:What to remember`,
    textMissing: $localize`:@@memory.textMissing:Write something, or forget the memory instead.`,
    cancel: $localize`:@@memory.cancel:Cancel`,
    save: $localize`:@@memory.save:Save`,
    forgetHeading: $localize`:@@memory.forgetHeading:Forget this memory?`,
    forgetMessage: $localize`:@@memory.forgetMessage:It is erased at once and cannot be brought back. Your chats keep what was said in them.`,
    forgetChat: $localize`:@@memory.forgetChat:Also delete the chat it was saved in`,
    emptyHeading: $localize`:@@memory.emptyHeading:Nothing remembered yet`,
    emptyText: $localize`:@@memory.emptyText:When you ask glave to remember something, it shows up here.`,
    offHeading: $localize`:@@memory.offHeading:Memory is off`,
    offText: $localize`:@@memory.offText:Turn it on with memory = "on" in config.toml (glave policy explain shows it).`,
    untrusted: $localize`:@@memory.untrusted:saved after reading untrusted content`,
  } as const;
  protected readonly icon = Brain;
  protected readonly textRows = 3;
  protected readonly service = inject(MemoryService);
  // The validator is passed as itself: Glaze's required mark compares it by reference.
  /* eslint-disable @typescript-eslint/unbound-method -- see above */
  protected readonly text = new FormControl('', {
    nonNullable: true,
    validators: [Validators.required],
  });
  /* eslint-enable @typescript-eslint/unbound-method */
  protected readonly editing = signal<Remembered | null>(null);
  protected readonly forgetting = signal<Remembered | null>(null);
  protected readonly editOpen = signal(false);
  protected readonly forgetOpen = signal(false);
  /** Whether the chat the memory being forgotten was saved in goes too. */
  protected readonly forgetChat = signal(false);
  protected readonly textMissing = signal(false);
  /** Whether the memory being forgotten was saved in a chat, which can go with it. */
  protected readonly hasChat = computed(() => (this.forgetting()?.source ?? null) !== null);

  protected readonly rows = computed(() =>
    this.service.memories().map((memory) => ({
      memory,
      id: memory.memory,
      text: memory.text,
      scope: memory.scope === 'personal' ? this.copy.personal : this.copy.project,
      untrusted: memory.untrusted_read === true,
      editLabel: $localize`:@@memory.editLabel:Edit: ${memory.text}:MEMORY:`,
      forgetLabel: $localize`:@@memory.forgetLabel:Forget: ${memory.text}:MEMORY:`,
    })),
  );
  /** What went wrong last, in the server's words. */
  protected readonly problem = computed(() => {
    const error = this.service.error();
    return error === null
      ? null
      : $localize`:@@memory.failed:Something went wrong: ${error}:ERROR:`;
  });
  protected readonly off = computed(() => this.service.state() === 'off');
  protected readonly ready = computed(() => this.service.state() === 'ready');

  constructor() {
    void this.service.refresh();
  }

  protected edit(memory: Remembered): void {
    this.editing.set(memory);
    this.text.reset(memory.text);
    this.textMissing.set(false);
    this.editOpen.set(true);
  }

  protected cancelEdit(): void {
    this.editOpen.set(false);
  }

  protected async save(): Promise<void> {
    const memory = this.editing();
    this.text.markAsTouched();
    this.textMissing.set(this.text.hasError('required'));
    if (memory === null || this.text.invalid) {
      return;
    }
    this.editOpen.set(false);
    await this.service.edit(memory.memory, this.text.value.trim());
  }

  protected forget(memory: Remembered): void {
    this.forgetting.set(memory);
    this.forgetChat.set(false);
    this.forgetOpen.set(true);
  }

  protected async decided(yes: boolean): Promise<void> {
    const memory = this.forgetting();
    if (yes && memory !== null) {
      await this.service.forget(memory.memory, this.hasChat() && this.forgetChat());
    }
  }
}
