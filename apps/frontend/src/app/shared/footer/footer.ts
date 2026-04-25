import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink } from '@angular/router';

@Component({
	selector: 'glave-footer',
	imports: [RouterLink],
	templateUrl: './footer.html',
	styleUrl: './footer.scss',
	changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Footer {
	protected readonly year = new Date().getFullYear();
}
