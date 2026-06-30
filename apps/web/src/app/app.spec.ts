import { TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import {
  HttpTestingController,
  provideHttpClientTesting,
} from '@angular/common/http/testing';
import { App } from './app';

describe('App', () => {
  let httpMock: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [provideHttpClient(), provideHttpClientTesting()],
    }).compileComponents();
    httpMock = TestBed.inject(HttpTestingController);
  });

  it('should create the app', () => {
    const fixture = TestBed.createComponent(App);
    expect(fixture.componentInstance).toBeTruthy();
  });

  it('should show the calm loading state before auth resolves', () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('.gate-loading')).toBeTruthy();
    httpMock.expectOne('/auth/me').flush(null, { status: 401, statusText: 'Unauthorized' });
  });

  it('should show the login when not authenticated', async () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    httpMock
      .expectOne('/auth/me')
      .flush(null, { status: 401, statusText: 'Unauthorized' });
    await fixture.whenStable();
    fixture.detectChanges();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('app-login')).toBeTruthy();
  });

  it('should show the Hope chat when authenticated', async () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    httpMock.expectOne('/auth/me').flush({ id: '1', email: 'a@b.c' });
    await fixture.whenStable();
    fixture.detectChanges();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('app-hope-chat')).toBeTruthy();
    expect(compiled.querySelector('.say--hope')?.textContent).toContain('Hope');
  });

  afterEach(() => {
    httpMock.verify();
  });
});
