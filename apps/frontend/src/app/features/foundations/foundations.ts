import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormControl, ReactiveFormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatIconButton } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatChipsModule } from '@angular/material/chips';
import { MatDialog, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatDividerModule } from '@angular/material/divider';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatRadioModule } from '@angular/material/radio';
import { MatSelectModule } from '@angular/material/select';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatTableModule } from '@angular/material/table';
import { MatTabsModule } from '@angular/material/tabs';
import { MatTooltipModule } from '@angular/material/tooltip';

interface Article {
	section: string;
	title: string;
	scope: 'binding' | 'guidance';
	updated: string;
}

const ARTICLES: Article[] = [
	{ section: 'I', title: 'Primacy of Human Agency', scope: 'binding', updated: 'Apr 2026' },
	{ section: 'II', title: 'Hierarchy of Intent', scope: 'binding', updated: 'Apr 2026' },
	{ section: 'III', title: 'Stewardship and Exchange', scope: 'binding', updated: 'Apr 2026' },
	{ section: 'IV', title: 'Architectural Alignment', scope: 'binding', updated: 'Apr 2026' },
	{ section: 'V', title: 'Operational Conduct', scope: 'guidance', updated: 'Apr 2026' },
	{
		section: 'VI',
		title: 'Enforcement and Accountability',
		scope: 'binding',
		updated: 'Apr 2026',
	},
];

@Component({
	selector: 'glave-foundations',
	imports: [
		ReactiveFormsModule,
		MatButtonModule,
		MatIconButton,
		MatCardModule,
		MatCheckboxModule,
		MatChipsModule,
		MatDialogModule,
		MatDividerModule,
		MatFormFieldModule,
		MatIconModule,
		MatInputModule,
		MatMenuModule,
		MatProgressBarModule,
		MatProgressSpinnerModule,
		MatRadioModule,
		MatSelectModule,
		MatSlideToggleModule,
		MatSnackBarModule,
		MatTableModule,
		MatTabsModule,
		MatTooltipModule,
	],
	templateUrl: './foundations.html',
	styleUrl: './foundations.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export default class Foundations {
	private readonly dialog = inject(MatDialog);
	private readonly snack = inject(MatSnackBar);

	protected readonly articles = ARTICLES;
	protected readonly displayedColumns = [
		'section',
		'title',
		'scope',
		'updated',
		'actions',
	] as const;

	protected readonly demoText = new FormControl('A glave is the instrument, not the wielder.');
	protected readonly demoSelect = new FormControl<'augment' | 'automate' | 'amplify'>('augment');
	protected readonly demoCheckbox = new FormControl(true);
	protected readonly demoToggle = new FormControl(false);
	protected readonly demoRadio = new FormControl<'light' | 'dark' | 'system'>('system');

	protected readonly counter = signal(0);

	protected readonly typeScale = [
		{ token: 'display-large', sample: 'Empower the human core' },
		{ token: 'display-medium', sample: 'Augmentation over replacement' },
		{ token: 'display-small', sample: 'Stewardship of the commons' },
		{ token: 'headline-large', sample: 'Article I — Primacy of Agency' },
		{ token: 'headline-medium', sample: 'Section 1. Preservation' },
		{ token: 'title-large', sample: 'Foundations' },
		{
			token: 'body-large',
			sample: 'Profit is recognized solely as the instrument of independence.',
		},
		{
			token: 'body-medium',
			sample: 'Body medium — running paragraph copy in long-form documents.',
		},
		{ token: 'label-large', sample: 'LABEL · LARGE' },
	];

	protected readonly palette = [
		{ name: 'Iron 700', token: '--glave-iron-700' },
		{ name: 'Iron 600', token: '--glave-iron-600' },
		{ name: 'Iron 500', token: '--glave-iron-500' },
		{ name: 'Iron 300', token: '--glave-iron-300' },
		{ name: 'Copper 700', token: '--glave-copper-700' },
		{ name: 'Copper 600', token: '--glave-copper-600' },
		{ name: 'Copper 500', token: '--glave-copper-500' },
		{ name: 'Copper 300', token: '--glave-copper-300' },
	];

	protected readonly graphite = [
		{ name: '950', token: '--glave-graphite-950' },
		{ name: '900', token: '--glave-graphite-900' },
		{ name: '800', token: '--glave-graphite-800' },
		{ name: '700', token: '--glave-graphite-700' },
		{ name: '600', token: '--glave-graphite-600' },
		{ name: '500', token: '--glave-graphite-500' },
		{ name: '400', token: '--glave-graphite-400' },
		{ name: '300', token: '--glave-graphite-300' },
		{ name: '200', token: '--glave-graphite-200' },
		{ name: '100', token: '--glave-graphite-100' },
		{ name: '50', token: '--glave-graphite-50' },
	];

	protected readonly systemColors = [
		{ name: 'primary', token: '--mat-sys-primary' },
		{ name: 'on-primary', token: '--mat-sys-on-primary' },
		{ name: 'primary-container', token: '--mat-sys-primary-container' },
		{ name: 'tertiary', token: '--mat-sys-tertiary' },
		{ name: 'tertiary-container', token: '--mat-sys-tertiary-container' },
		{ name: 'surface', token: '--mat-sys-surface' },
		{ name: 'surface-container', token: '--mat-sys-surface-container' },
		{ name: 'surface-container-high', token: '--mat-sys-surface-container-high' },
		{ name: 'outline', token: '--mat-sys-outline' },
		{ name: 'outline-variant', token: '--mat-sys-outline-variant' },
		{ name: 'error', token: '--mat-sys-error' },
	];

	protected openDialog(): void {
		this.dialog.open(FoundationsDialog, {
			width: '480px',
			autoFocus: 'first-tabbable',
		});
	}

	protected showSnack(): void {
		this.snack.open('Saved. Audit log updated.', 'Dismiss', {
			duration: 3000,
		});
	}

	protected increment(): void {
		this.counter.update((v) => v + 1);
	}
}

@Component({
	selector: 'glave-foundations-dialog',
	imports: [MatButtonModule, MatDialogModule],
	template: `
		<h2 mat-dialog-title>Confirm publication</h2>
		<mat-dialog-content>
			Publishing the audit makes it part of the public record. This action is appended, never
			overwritten — prior versions remain accessible.
		</mat-dialog-content>
		<mat-dialog-actions align="end">
			<button matButton (click)="ref.close()">Cancel</button>
			<button matButton="filled" (click)="ref.close('confirm')">Publish</button>
		</mat-dialog-actions>
	`,
	changeDetection: ChangeDetectionStrategy.OnPush,
})
class FoundationsDialog {
	protected readonly ref = inject(MatDialogRef);
}
