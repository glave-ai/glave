import { Location } from '@angular/common';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { BackService, NAVIGATION } from './back.service';

function setUp(navigation: Pick<Navigation, 'canGoBack'> | null): {
  readonly service: BackService;
  readonly back: ReturnType<typeof vi.fn>;
  readonly navigate: ReturnType<typeof vi.fn>;
} {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), { provide: NAVIGATION, useValue: navigation }],
  });
  const back = vi.fn();
  const navigate = vi.fn().mockResolvedValue(true);
  vi.spyOn(TestBed.inject(Location), 'back').mockImplementation(back);
  vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockImplementation(navigate);
  return { service: TestBed.inject(BackService), back, navigate };
}

describe('BackService', () => {
  it('goes back to the page before when it is glave’s', async () => {
    const { service, back, navigate } = setUp({ canGoBack: true });
    await service.back('/');
    expect(back).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('goes to the page it belongs under when the person arrived directly', async () => {
    const { service, back, navigate } = setUp({ canGoBack: false });
    await service.back('/settings');
    expect(navigate).toHaveBeenCalledWith('/settings');
    expect(back).not.toHaveBeenCalled();
  });

  it('goes to the page it belongs under in a browser that keeps no such history', async () => {
    const { service, navigate } = setUp(null);
    await service.back('/');
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
