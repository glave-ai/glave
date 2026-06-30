import { Component, inject, OnInit } from '@angular/core';
import { AuthService } from './auth';
import { HopeChat } from './hope-chat';
import { Login } from './login';

@Component({
  selector: 'app-root',
  imports: [Login, HopeChat],
  templateUrl: './app.html',
  styleUrl: './app.scss',
})
export class App implements OnInit {
  private readonly auth = inject(AuthService);

  protected readonly status = this.auth.status;

  ngOnInit(): void {
    void this.auth.refresh();
  }
}
