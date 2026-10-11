import { TestBed } from '@angular/core/testing';

import { injectId } from './ids';

describe('injectId', () => {
  it('gives each caller a new id with its prefix', () => {
    const [first, second] = TestBed.runInInjectionContext(() => [
      injectId('glz-field'),
      injectId('glz-field'),
    ]);
    expect(first).toMatch(/^glz-field-\d+$/);
    expect(second).not.toBe(first);
  });
});
