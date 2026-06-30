import { inject, Injectable, signal } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

export interface User {
  readonly id: string;
  readonly email: string;
}

export type AuthStatus = 'loading' | 'authed' | 'anon';

interface OtpStartResponse {
  readonly sent: boolean;
  readonly dev_code: string | null;
}

/// Passwordless session auth against the Glave backend. The session is an
/// HttpOnly cookie (`glave_session`) the browser never reads — every call is
/// same-origin with `withCredentials: true` so the cookie rides along.
@Injectable({ providedIn: 'root' })
export class AuthService {
  private readonly http = inject(HttpClient);

  readonly user = signal<User | null>(null);
  readonly status = signal<AuthStatus>('loading');

  /// Ask the server who we are. Called once on app start.
  async refresh(): Promise<void> {
    try {
      const me = await firstValueFrom(
        this.http.get<User>('/auth/me', { withCredentials: true }),
      );
      this.user.set(me);
      this.status.set('authed');
    } catch {
      this.user.set(null);
      this.status.set('anon');
    }
  }

  /// Step one: ask Hope to send a code to this email.
  async startOtp(email: string): Promise<{ sent: boolean; devCode: string | null }> {
    const res = await firstValueFrom(
      this.http.post<OtpStartResponse>(
        '/auth/otp/start',
        { email },
        { withCredentials: true },
      ),
    );
    return { sent: res.sent, devCode: res.dev_code };
  }

  /// Step two: hand back the code to claim the session cookie.
  async verifyOtp(email: string, code: string): Promise<void> {
    const me = await firstValueFrom(
      this.http.post<User>(
        '/auth/otp/verify',
        { email, code },
        { withCredentials: true },
      ),
    );
    this.user.set(me);
    this.status.set('authed');
  }

  /// Drop the session and return to the welcome.
  async logout(): Promise<void> {
    await firstValueFrom(
      this.http.post('/auth/logout', null, { withCredentials: true }),
    );
    this.user.set(null);
    this.status.set('anon');
  }
}
