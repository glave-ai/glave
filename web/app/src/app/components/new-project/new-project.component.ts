import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  model,
  output,
  signal,
  untracked,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzIconComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
  injectId,
} from '@glave/glaze';
import { Folder } from 'lucide';

import { ACCOUNTS_SECTION } from '../../lib/credentials';
import {
  NAME_LIMIT,
  type ProjectDraft,
  type ProjectStart,
  createParams,
  fallbackName,
  isProjectStart,
  projectProblem,
} from '../../lib/projects';
import { CredentialsService } from '../../services/credentials.service';
import { SessionService } from '../../services/session.service';
import { ProjectsService } from '../../services/projects.service';
import { FolderPickerComponent } from '../folder-picker/folder-picker.component';

const COPY = {
  heading: 'New project',
  name: 'Name',
  start: 'Start from',
  folder: 'Folder',
  change: 'Change',
  address: 'Repository address',
  addressExample: 'https://github.com/owner/repository',
  privateConnected: 'Private repositories use the git host you connected.',
  privateNeeds: 'Private repositories need a connected git host:',
  connect: 'connect one in Settings',
  cloning: 'Cloning the repository. A large one can take a few minutes.',
  created: 'Created the project',
  noFolder: 'Its threads start in an empty folder for now; a computer for it comes later.',
  cancel: 'Cancel',
  create: 'Create',
  creating: 'Creating…',
  cloningButton: 'Cloning…',
} as const;

/** The three places a project can start from, as the person picks one. */
const STARTS: readonly {
  readonly value: ProjectStart;
  readonly label: string;
  readonly hint: string;
}[] = [
  {
    value: 'folder',
    label: 'A folder on this computer',
    hint: 'Work on files you already have.',
  },
  {
    value: 'repository',
    label: 'A repository',
    hint: 'Clone one from GitHub or another git host.',
  },
  { value: 'empty', label: 'Empty', hint: 'glave makes a new folder for it.' },
];

/**
 * New project: a name, and for the person on the computer glave runs on, where its files come
 * from: a folder there (picked from the folders the server lists), a repository it clones
 * there, or a new, empty folder. Anyone else makes a project with no folder, by name alone: the
 * server would refuse them a folder, so none is offered. The name is the folder's or the
 * repository's, following it until the person types in the field. Making it goes straight into
 * the project, at the home screen with the project chosen for the first message (the composer
 * takes focus there), and says the name the server gave it.
 *
 * It starts afresh each time it opens, except while a project is being made, after one could
 * not be made, and after the person went to connect a git host: then it is as they left it.
 */
@Component({
  selector: 'app-new-project',
  imports: [
    FolderPickerComponent,
    RouterLink,
    GlzButtonComponent,
    GlzDialogComponent,
    GlzFormFieldComponent,
    GlzIconComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
  ],
  templateUrl: './new-project.component.html',
  styleUrl: './new-project.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NewProjectComponent {
  /** Whether the dialog is open. */
  readonly open = model(false);
  /** Emitted when the person follows a link from it, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = COPY;
  protected readonly starts = STARTS;
  protected readonly limit = NAME_LIMIT;
  protected readonly icons = { folder: Folder };
  protected readonly settingsLink = '/settings';
  protected readonly accountsSection = ACCOUNTS_SECTION;
  protected readonly formId = injectId('new-project-form');
  protected readonly folderLabelId = injectId('new-project-folder');
  protected readonly cloningId = injectId('new-project-cloning');
  private readonly session = inject(SessionService);
  private readonly projects = inject(ProjectsService);
  private readonly credentials = inject(CredentialsService);
  private readonly router = inject(Router);
  private readonly announcer = inject(LiveAnnouncer);
  /** Whether the next time it opens keeps what the person had. */
  private keep = false;

  /** Whether the person is on the computer glave runs on, where a project's folder would be. */
  protected readonly local = this.session.local;
  protected readonly start = signal<ProjectStart>('folder');
  /** The folder picked; while there is none, the picker shows. */
  protected readonly folder = signal<string | null>(null);
  /** Where the picker opens: the home folder, or the folder picked before, to change it. */
  protected readonly pickFrom = signal<string | null>(null);
  protected readonly url = signal('');
  protected readonly creating = signal(false);
  protected readonly problem = signal<string | null>(null);
  /** The name as the person typed it, once they have typed in the field. */
  private readonly typed = signal<string | null>(null);

  /** Where its files come from. */
  private readonly source = computed<Omit<ProjectDraft, 'name'>>(() => ({
    start: this.local() ? this.start() : null,
    folder: this.folder(),
    url: this.url(),
  }));
  /** The name: the folder's or the repository's, until the person types one of their own. */
  protected readonly name = computed(() => this.typed() ?? fallbackName(this.source()));
  private readonly draft = computed<ProjectDraft>(() => ({ ...this.source(), name: this.name() }));
  private readonly params = computed(() => createParams(this.draft()));
  protected readonly fromFolder = computed(() => this.draft().start === 'folder');
  protected readonly fromRepository = computed(() => this.draft().start === 'repository');
  protected readonly connected = computed(() => this.credentials.accounts().length > 0);
  protected readonly cloning = computed(() => this.creating() && this.fromRepository());
  protected readonly incomplete = computed(() => this.params() === null);
  /** While it is being made, Create stays focusable but does nothing. */
  protected readonly createBusy = computed(() => (this.creating() ? 'true' : null));
  protected readonly createLabel = computed(() => {
    if (this.cloning()) {
      return COPY.cloningButton;
    }
    return this.creating() ? COPY.creating : COPY.create;
  });

  constructor() {
    // Each time it opens it starts afresh, or as the person left it (see the class's docs).
    effect(() => {
      if (this.open()) {
        untracked(() => {
          this.reset();
        });
      }
    });
  }

  protected typedName(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.typed.set(event.target.value);
    }
  }

  protected typedUrl(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.url.set(event.target.value);
    }
  }

  protected pickStart(value: string | null): void {
    if (isProjectStart(value)) {
      this.start.set(value);
      this.problem.set(null);
    }
  }

  protected picked(path: string): void {
    this.folder.set(path);
    this.problem.set(null);
  }

  /** Shows the picker again, at the folder picked, so another can be chosen. */
  protected change(): void {
    this.pickFrom.set(this.folder());
    this.folder.set(null);
  }

  protected cancel(): void {
    this.keep = false;
    this.open.set(false);
  }

  /**
   * Follows the link to Settings: the dialog closes, and so does a drawer around it, keeping
   * what the person had for when they come back.
   */
  protected followed(): void {
    this.keep = true;
    this.open.set(false);
    this.navigated.emit();
  }

  /**
   * Makes the project and, if the dialog is still open, goes into it: the home screen, with
   * the project chosen for the first message. The server's own words say what went wrong.
   */
  protected async create(event?: Event): Promise<void> {
    event?.preventDefault();
    const params = this.params();
    if (params === null || this.creating()) {
      return;
    }
    this.creating.set(true);
    this.problem.set(null);
    if (this.cloning()) {
      void this.announcer.announce(COPY.cloning);
    }
    try {
      const project = await this.projects.create(params);
      this.keep = false;
      void this.announcer.announce(`${COPY.created} ${project.name}`);
      if (this.open()) {
        // A drawer closes first, so the composer can take focus once the page is shown.
        this.open.set(false);
        this.navigated.emit();
        await this.router.navigate(['/'], { queryParams: { project: project.id } });
      }
    } catch (error: unknown) {
      this.keep = true;
      this.problem.set(projectProblem(error));
    } finally {
      this.creating.set(false);
    }
  }

  /** Starts afresh, unless what the person had is kept; reads their git hosts again either way. */
  private reset(): void {
    if (this.local()) {
      void this.credentials.load();
    }
    const kept = this.keep || this.creating();
    this.keep = false;
    if (kept) {
      return;
    }
    this.typed.set(null);
    this.start.set('folder');
    this.folder.set(null);
    this.pickFrom.set(null);
    this.url.set('');
    this.problem.set(null);
  }
}
