import { inject, Injectable } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

export interface ChatMessage {
  readonly role: 'system' | 'user' | 'assistant';
  readonly content: string;
}

interface ChatResponse {
  readonly choices: readonly { readonly message: { readonly content: string } }[];
}

/// Talks to the Glave API (OpenAI-compatible). Same-origin and cookie-authed:
/// the session cookie (`glave_session`) rides along via `withCredentials`, so no
/// secret lives in the browser.
@Injectable({ providedIn: 'root' })
export class HopeService {
  private readonly http = inject(HttpClient);

  async reply(history: readonly ChatMessage[]): Promise<string> {
    const res = await firstValueFrom(
      this.http.post<ChatResponse>(
        '/v1/chat/completions',
        { model: 'hope', messages: [...history], stream: false },
        { withCredentials: true },
      ),
    );
    return res.choices[0]?.message.content ?? '';
  }
}
