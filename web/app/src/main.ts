import { bootstrapApplication } from '@angular/platform-browser';

import { appConfig } from './app/app.config';
import { AppComponent } from './app/components/app/app.component';

bootstrapApplication(AppComponent, appConfig).catch((error: unknown) => {
  // Nothing else can report a failure to start; the browser console is where it shows.
  // eslint-disable-next-line no-console -- see above
  console.error(error);
});
