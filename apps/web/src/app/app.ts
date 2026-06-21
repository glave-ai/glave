import { Component, signal } from '@angular/core';
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
  protected readonly draft = signal('');
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
}
