import { ChangeDetectionStrategy, Component } from '@angular/core';
import { NgOptimizedImage } from '@angular/common';
import { RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';

interface Tile {
	href: string;
	eyebrow: string;
	title: string;
	blurb: string;
	image: string;
	alt: string;
}

@Component({
	selector: 'glave-home',
	imports: [NgOptimizedImage, RouterLink, MatButtonModule],
	templateUrl: './home.html',
	styleUrl: './home.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export default class Home {
	protected readonly tiles: readonly Tile[] = [
		{
			href: '/api',
			eyebrow: 'API',
			title: 'Build with Glave',
			blurb: 'A transparent API for affordable intelligence. Pricing you can read, behavior you can audit.',
			image: '/images/tile-api.webp',
			alt: 'A drafting set on graphite paper with iron-blue construction lines and a copper protractor.',
		},
		{
			href: '/chat',
			eyebrow: 'Chat',
			title: 'Think with Glave',
			blurb: 'A conversational surface designed to help you draft, research, and decide — not to do it for you.',
			image: '/images/tile-chat.webp',
			alt: 'Two open notebooks with handwritten iron-blue notes and a copper bookmark.',
		},
		{
			href: '/docs',
			eyebrow: 'Docs',
			title: 'Read the manuals',
			blurb: 'Guides, references, and the engineering rationale behind every choice we shipped.',
			image: '/images/tile-docs.webp',
			alt: 'A short stack of bound technical manuals with iron-blue cloth spines and copper title plates.',
		},
		{
			href: '/blog',
			eyebrow: 'Blog',
			title: 'Notes from the work',
			blurb: 'Research updates, product writing, and audit reports — in plain language.',
			image: '/images/tile-blog.webp',
			alt: 'A workshop drying rack with freshly-pulled prints in soft afternoon light.',
		},
	];
}
