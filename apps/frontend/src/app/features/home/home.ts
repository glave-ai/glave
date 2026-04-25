import { ChangeDetectionStrategy, Component } from '@angular/core';
import { NgOptimizedImage } from '@angular/common';
import { RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { RevealOnScrollDirective } from '../../shared/reveal-on-scroll.directive';

interface Tile {
	number: string;
	href: string;
	eyebrow: string;
	title: string;
	blurb: string;
	image: string;
	alt: string;
}

interface Note {
	number: string;
	date: string;
	title: string;
	blurb: string;
}

@Component({
	selector: 'glave-home',
	imports: [NgOptimizedImage, RouterLink, MatButtonModule, RevealOnScrollDirective],
	templateUrl: './home.html',
	styleUrl: './home.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export default class Home {
	protected readonly tiles: readonly Tile[] = [
		{
			number: '01',
			href: '/api',
			eyebrow: 'API',
			title: 'Build with Glave',
			blurb: 'A transparent API for affordable intelligence. Pricing you can read, behavior you can audit.',
			image: '/images/tile-api.webp',
			alt: 'A drafting set on graphite paper with iron-blue construction lines and a copper protractor.',
		},
		{
			number: '02',
			href: '/chat',
			eyebrow: 'Chat',
			title: 'Think with Glave',
			blurb: 'A conversational surface designed to help you draft, research, and decide — not to do it for you.',
			image: '/images/tile-chat.webp',
			alt: 'Two open notebooks with handwritten iron-blue notes and a copper bookmark.',
		},
		{
			number: '03',
			href: '/docs',
			eyebrow: 'Docs',
			title: 'Read the manuals',
			blurb: 'Guides, references, and the engineering rationale behind every choice we shipped.',
			image: '/images/tile-docs.webp',
			alt: 'A short stack of bound technical manuals with iron-blue cloth spines and copper title plates.',
		},
		{
			number: '04',
			href: '/blog',
			eyebrow: 'Blog',
			title: 'Notes from the work',
			blurb: 'Research updates, product writing, and audit reports — in plain language.',
			image: '/images/tile-blog.webp',
			alt: 'A workshop drying rack with freshly-pulled prints in soft afternoon light.',
		},
	];

	protected readonly notes: readonly Note[] = [
		{
			number: 'No. 03',
			date: 'Nov 2025',
			title: 'On transparent pricing.',
			blurb: 'Why every line of our price sheet must be readable by the people paying for it.',
		},
		{
			number: 'No. 02',
			date: 'Oct 2025',
			title: 'Why we publish failure modes.',
			blurb: 'A model is only as honest as the limits it admits. A short note on what we disclose, and why.',
		},
		{
			number: 'No. 01',
			date: 'Sep 2025',
			title: 'Open foundations: what we mean.',
			blurb: 'Portability, customizability, control — three commitments we are willing to be measured against.',
		},
	];
}
