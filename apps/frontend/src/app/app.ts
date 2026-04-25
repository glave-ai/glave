import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { Footer } from './shared/footer/footer';

@Component({
	selector: 'glave-root',
	imports: [RouterOutlet, RouterLink, RouterLinkActive, Footer],
	templateUrl: './app.html',
	styleUrl: './app.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {}
