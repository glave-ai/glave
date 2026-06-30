import { Component, inject, signal } from '@angular/core';
import { DOCUMENT } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { AuthService } from './auth';
import { HopeService } from './hope';

interface Message {
  readonly from: 'hope' | 'you';
  readonly text: string;
}

@Component({
  selector: 'app-hope-chat',
  imports: [MatFormFieldModule, MatInputModule, MatButtonModule],
  templateUrl: './hope-chat.html',
  styleUrl: './hope-chat.scss',
})
export class HopeChat {
  private readonly doc = inject(DOCUMENT);
  private readonly hope = inject(HopeService);
  private readonly auth = inject(AuthService);

  protected readonly draft = signal('');
  protected readonly sending = signal(false);
  protected readonly dark = signal(this.prefersDark());
  protected readonly messages = signal<readonly Message[]>([
    { from: 'hope', text: "Hey — I'm Hope. What's on your mind?" },
  ]);

  protected async send(): Promise<void> {
    const text = this.draft().trim();
    if (!text || this.sending()) {
      return;
    }
    this.draft.set('');
    this.messages.update((current) => [...current, { from: 'you', text }]);
    this.sending.set(true);
    try {
      const history = this.messages().map((m) => ({
        role: m.from === 'you' ? ('user' as const) : ('assistant' as const),
        content: m.text,
      }));
      const answer = await this.hope.reply(history);
      this.messages.update((current) => [
        ...current,
        { from: 'hope', text: answer.trim() || '…' },
      ]);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : 'Something went wrong reaching Hope.';
      this.messages.update((current) => [...current, { from: 'hope', text: `(${message})` }]);
    } finally {
      this.sending.set(false);
    }
  }

  protected toggleTheme(): void {
    const next = !this.dark();
    this.dark.set(next);
    this.doc.documentElement.setAttribute('data-theme', next ? 'dark' : 'light');
  }

  protected async logout(): Promise<void> {
    await this.auth.logout();
  }

  private prefersDark(): boolean {
    return this.doc.defaultView?.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false;
  }
}
