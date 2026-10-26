import { Service } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { Setting } from '@glave/client';

import { ChangesService, onChange } from './changes.service';

/** A service that reads its setting again each time it changes. */
@Service()
class Reader {
  reads: Setting[] = [];

  constructor() {
    onChange('search', () => {
      this.reads.push('search');
    });
  }
}

describe('ChangesService', () => {
  it('counts each change the server tells of, for each setting', () => {
    const changes = TestBed.inject(ChangesService);
    expect(changes.times('search')).toBe(0);
    changes.tell('search');
    changes.tell('search');
    changes.tell('grouping');
    expect(changes.times('search')).toBe(2);
    expect(changes.times('grouping')).toBe(1);
    expect(changes.times('providers')).toBe(0);
  });

  it('reads a setting again each time it changes from then on, and for no other', () => {
    const changes = TestBed.inject(ChangesService);
    changes.tell('search');
    const reader = TestBed.inject(Reader);
    TestBed.tick();
    expect(reader.reads).toEqual([]);
    changes.tell('grouping');
    TestBed.tick();
    expect(reader.reads).toEqual([]);
    changes.tell('search');
    TestBed.tick();
    changes.tell('search');
    TestBed.tick();
    expect(reader.reads).toEqual(['search', 'search']);
  });
});
