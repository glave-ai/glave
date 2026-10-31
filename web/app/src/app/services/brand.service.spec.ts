import { TestBed } from '@angular/core/testing';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { AppearanceService } from './appearance.service';
import { BrandService } from './brand.service';

const PNG = btoa('\x89PNG');

function look(logo?: string): object {
  return {
    organization: { name: 'Acme', accent: 'plain', locked: false, ...(logo ? { logo } : {}) },
  };
}

async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
}

describe('BrandService', () => {
  it('reads each logo once', async () => {
    const fake = new FakeSession()
      .answer('appearance/get', look('one'))
      .answer('brand/logo/get', { logo: { sha256: 'one', media_type: 'image/png', data: PNG } });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const brand = TestBed.inject(BrandService);
    await settle();
    expect(brand.logoUrl()).toMatch(/^blob:/u);
    expect(fake.paramsOf('brand/logo/get')).toHaveLength(1);

    await TestBed.inject(AppearanceService).read();
    await settle();
    expect(fake.paramsOf('brand/logo/get')).toHaveLength(1);

    fake.answer('appearance/get', look());
    await TestBed.inject(AppearanceService).read();
    await settle();
    expect(brand.logoUrl()).toBeNull();
  });

  it('has none where no organization is shown', async () => {
    const fake = new FakeSession();
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const brand = TestBed.inject(BrandService);
    await settle();
    expect(brand.logoUrl()).toBeNull();
    expect(fake.paramsOf('brand/logo/get')).toEqual([]);
  });
});
