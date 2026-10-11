import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterOutlet } from '@angular/router';
import { GlzToastStackComponent } from '@glave/glaze';

/** The app's root: the page the router shows, and the toasts over it. */
@Component({
  selector: 'app-root',
  imports: [RouterOutlet, GlzToastStackComponent],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppComponent {}
