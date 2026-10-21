import { TestBed } from '@angular/core/testing';

import { GlzWorkingComponent } from '../components/working/working.component';
import { GLZ_TEXT, type GlzText } from '../tokens/text';
import { glzText } from './words';

describe('glzText', () => {
  it('gives a part’s English unless the app gives its words', () => {
    expect(TestBed.runInInjectionContext(() => glzText('working', { label: 'Working…' }))).toEqual({
      label: 'Working…',
    });
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({
      providers: [{ provide: GLZ_TEXT, useValue: { working: { label: 'Au travail…' } } }],
    });
    expect(TestBed.runInInjectionContext(() => glzText('working', { label: 'Working…' }))).toEqual({
      label: 'Au travail…',
    });
  });

  it('puts the app’s words in a component, which its own input still replaces', async () => {
    const french: Partial<GlzText> = { working: { label: 'Au travail…' } };
    TestBed.configureTestingModule({ providers: [{ provide: GLZ_TEXT, useValue: french }] });
    const fixture = TestBed.createComponent(GlzWorkingComponent);
    await fixture.whenStable();
    expect(fixture.componentInstance.label()).toBe('Au travail…');
    fixture.componentRef.setInput('label', 'Réflexion…');
    await fixture.whenStable();
    expect(fixture.componentInstance.label()).toBe('Réflexion…');
  });
});
