import { provideBrowserGlobalErrorListeners } from '@angular/core';
import { bootstrapApplication } from '@angular/platform-browser';

import { ShowcaseComponent } from './components/showcase/showcase.component';

// A failure to start shows in the browser's console, as an unhandled rejection.
void bootstrapApplication(ShowcaseComponent, { providers: [provideBrowserGlobalErrorListeners()] });
