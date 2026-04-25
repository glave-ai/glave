import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink } from '@angular/router';

@Component({
	selector: 'glave-legal',
	imports: [RouterLink],
	template: `
		<section class="page">
			<p class="eyebrow">Documents</p>
			<h1>Legal</h1>
			<p class="lede">
				Glave's binding documents and standard legal surfaces will be published here. Today
				this is a placeholder; the canonical text already lives in the
				<code>sacred/</code> directory of the source repository.
			</p>
			<ul class="doc-list">
				<li>
					<span class="doc-name">Manifesto</span><span class="doc-meta">ratified</span>
				</li>
				<li>
					<span class="doc-name">Constitution</span
					><span class="doc-meta">ratified draft</span>
				</li>
				<li>
					<span class="doc-name">Annual audit</span><span class="doc-meta">pending</span>
				</li>
				<li>
					<span class="doc-name">Privacy policy</span><span class="doc-meta">draft</span>
				</li>
				<li>
					<span class="doc-name">Terms of service</span
					><span class="doc-meta">draft</span>
				</li>
			</ul>
			<a routerLink="/" class="back">← Back to home</a>
		</section>
	`,
	styleUrl: './legal.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export default class Legal {}
