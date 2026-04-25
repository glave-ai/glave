import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink } from '@angular/router';

@Component({
	selector: 'glave-docs',
	imports: [RouterLink],
	template: `
		<section class="page">
			<p class="eyebrow">Coming soon</p>
			<h1>Docs</h1>
			<p class="lede">
				Documentation will live here — guides, API reference, model cards, and the
				engineering rationale behind the choices. We'll keep it terse, accurate, and honest
				about what each system does and does not do.
			</p>
			<a routerLink="/" class="back">← Back to home</a>
		</section>
	`,
	styles: `
		:host {
			display: block;
		}
		.page {
			max-width: 720px;
			margin: 0 auto;
			padding: 5rem 1.5rem 6rem;
			display: flex;
			flex-direction: column;
			gap: 1.25rem;
		}
		.eyebrow {
			font-size: 0.8125rem;
			font-weight: 600;
			letter-spacing: 0.08em;
			text-transform: uppercase;
			color: var(--mat-sys-primary);
			margin: 0;
		}
		h1 {
			font: var(--mat-sys-display-medium);
			margin: 0;
			letter-spacing: -0.025em;
			color: var(--mat-sys-on-surface);
		}
		.lede {
			font-size: 1.0625rem;
			line-height: 1.55;
			color: var(--mat-sys-on-surface-variant);
			margin: 0;
			max-width: 60ch;
		}
		.back {
			margin-top: 1rem;
			align-self: flex-start;
			font-size: 0.9375rem;
			color: var(--mat-sys-on-surface-variant);

			&:hover {
				color: var(--mat-sys-on-surface);
				text-decoration: none;
			}
		}
	`,
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export default class Docs {}
