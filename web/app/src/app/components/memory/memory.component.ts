import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import type { Remembered } from '@glave/client';
import {
  GlzBadgeComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzConfirmDialogComponent,
  GlzDialogComponent,
  GlzEmptyStateComponent,
  GlzErrorComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';
import { Brain } from 'lucide';

import { MemoryService } from '../../services/memory.service';

const COPY = {
  heading: 'Memory',
  intro:
    'What glave keeps between threads: about you, and about this space. Edit or forget any of it.',
  personal: 'about you',
  space: 'this space',
  edit: 'Edit',
  forget: 'Forget',
  editHeading: 'Edit the memory',
  text: 'What to remember',
  textMissing: 'Write something, or forget the memory instead.',
  cancel: 'Cancel',
  save: 'Save',
  forgetHeading: 'Forget this memory?',
  forgetMessage: 'Its key is destroyed at once; it cannot be brought back.',
  emptyHeading: 'Nothing remembered yet',
  emptyText: 'When you ask glave to remember something, it shows up here.',
  offHeading: 'Memory is off',
  offText: 'Turn it on with memory = "on" in config.toml (glave policy explain shows it).',
  failed: 'Something went wrong:',
  untrusted: 'saved after reading untrusted content',
} as const;

/** The memory page: each memory, with Edit and Forget. */
@Component({
  selector: 'app-memory',
  imports: [
    ReactiveFormsModule,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzCardComponent,
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
  protected readonly copy = COPY;
  protected readonly icon = Brain;
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
  protected readonly textMissing = signal(false);

  protected readonly rows = computed(() =>
    this.service.memories().map((memory) => ({
      memory,
      id: memory.memory,
      text: memory.text,
      scope: memory.scope === 'personal' ? COPY.personal : COPY.space,
      untrusted: memory.untrusted_read === true,
      editLabel: `${COPY.edit}: ${memory.text}`,
      forgetLabel: `${COPY.forget}: ${memory.text}`,
    })),
  );
  protected readonly off = computed(() => this.service.state() === 'off');
  protected readonly empty = computed(
    () => this.service.state() === 'ready' && this.service.memories().length === 0,
  );

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
    this.forgetOpen.set(true);
  }

  protected async decided(yes: boolean): Promise<void> {
    const memory = this.forgetting();
    if (yes && memory !== null) {
      await this.service.forget(memory.memory);
    }
  }
}
