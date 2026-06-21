import { Component, inject, signal } from '@angular/core';
import { DOCUMENT } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';

interface Message {
  readonly from: 'hope' | 'you';
  readonly text: string;
}

@Component({
  selector: 'app-root',
  imports: [MatFormFieldModule, MatInputModule, MatButtonModule],
  templateUrl: './app.html',
  styleUrl: './app.scss',
})
export class App {
  private readonly doc = inject(DOCUMENT);

  protected readonly draft = signal('');
  protected readonly dark = signal(this.prefersDark());
  protected readonly messages = signal<readonly Message[]>([
    { from: 'hope', text: "Hey — I'm Hope. What's on your mind?" },
  ]);

  protected send(): void {
    const text = this.draft().trim();
    if (!text) {
      return;
    }
    this.messages.update((current) => [
      ...current,
      { from: 'you', text },
      {
        from: 'hope',
        text: "I'm almost ready — once my brain's wired in, I'll really answer.",
      },
    ]);
    this.draft.set('');
  }

  protected toggleTheme(): void {
    const next = !this.dark();
    this.dark.set(next);
    this.doc.documentElement.setAttribute('data-theme', next ? 'dark' : 'light');
  }

  private prefersDark(): boolean {
    return this.doc.defaultView?.matchMedia('(prefers-color-scheme: dark)').matches ?? false;
  }
}
