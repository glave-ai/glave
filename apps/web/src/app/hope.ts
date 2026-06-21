import { inject, Injectable } from '@angular/core';
import { DOCUMENT } from '@angular/common';
import { HttpClient } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

export interface ChatMessage {
  readonly role: 'system' | 'user' | 'assistant';
  readonly content: string;
}

interface ChatResponse {
  readonly choices: readonly { readonly message: { readonly content: string } }[];
}

/// Talks to the Glave API (OpenAI-compatible). Base URL and dev key come from
/// localStorage so no secret lives in the code:
///   localStorage.glave_key  = "your-dev-key"   (matches the server's GLAVE_API_KEY)
///   localStorage.glave_base = "http://127.0.0.1:8080"   (optional override)
@Injectable({ providedIn: 'root' })
export class HopeService {
  private readonly http = inject(HttpClient);
  private readonly doc = inject(DOCUMENT);

  async reply(history: readonly ChatMessage[]): Promise<string> {
    const base = this.setting('glave_base', 'http://127.0.0.1:8080');
    const key = this.setting('glave_key', '');
    if (!key) {
      throw new Error(
        'No API key set yet. In the browser console: localStorage.glave_key = "your-dev-key"',
      );
    }
    const res = await firstValueFrom(
      this.http.post<ChatResponse>(
        `${base}/v1/chat/completions`,
        { model: 'hope', messages: [...history], stream: false },
        { headers: { Authorization: `Bearer ${key}` } },
      ),
    );
    return res.choices[0]?.message.content ?? '';
  }

  private setting(name: string, fallback: string): string {
    return this.doc.defaultView?.localStorage.getItem(name) ?? fallback;
  }
}
