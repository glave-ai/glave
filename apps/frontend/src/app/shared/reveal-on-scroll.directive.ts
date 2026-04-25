import {
	DestroyRef,
	Directive,
	ElementRef,
	OnInit,
	PLATFORM_ID,
	inject,
} from '@angular/core';
import { isPlatformBrowser } from '@angular/common';

@Directive({
	selector: '[glaveReveal]',
	host: {
		class: 'glave-reveal',
	},
})
export class RevealOnScrollDirective implements OnInit {
	private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
	private readonly destroyRef = inject(DestroyRef);
	private readonly platformId = inject(PLATFORM_ID);

	ngOnInit(): void {
		const el = this.host.nativeElement;

		if (!isPlatformBrowser(this.platformId)) {
			el.classList.add('is-visible');
			return;
		}

		const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		if (reduced || typeof IntersectionObserver === 'undefined') {
			el.classList.add('is-visible');
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (entry.isIntersecting) {
						el.classList.add('is-visible');
						observer.disconnect();
						break;
					}
				}
			},
			{ rootMargin: '0px 0px -10% 0px', threshold: 0.05 },
		);

		observer.observe(el);
		this.destroyRef.onDestroy(() => observer.disconnect());
	}
}
