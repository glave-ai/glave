import type { ModelInfo } from '@glave/client';

import { event } from '../testing/fixtures';
import { UNASKED, chosenEffort, effortOptions, effortToSend, levelName } from './effort';

const REASONS: ModelInfo = {
  id: 'openai/gpt-6-luna',
  name: 'GPT-6 Luna',
  default: true,
  reasoning: { efforts: ['xhigh', 'high', 'medium', 'low', 'none'], default: 'medium' },
};

describe('effort', () => {
  it('offers Default, named by the level it is, then exactly the levels the model lists', () => {
    expect(effortOptions(REASONS)).toEqual([
      { value: UNASKED, label: 'Default', hint: 'Medium', short: 'Medium' },
      { value: 'xhigh', label: 'Extra high' },
      { value: 'high', label: 'High' },
      { value: 'medium', label: 'Medium' },
      { value: 'low', label: 'Low' },
      { value: 'none', label: 'Off' },
    ]);
    expect(effortOptions({ id: 'm', name: 'M', default: false })).toEqual([]);
    expect(
      effortOptions({ id: 'm', name: 'M', default: false, reasoning: { mandatory: true } }),
    ).toEqual([]);
    expect(effortOptions(undefined)).toEqual([]);
    expect(levelName('turbo')).toBe('Turbo');
  });

  it('names Default by no level where none is sent unasked, which is the model maker’s way', () => {
    const unsent: ModelInfo = {
      id: 'm',
      name: 'M',
      default: false,
      reasoning: { efforts: ['high', 'low'] },
    };
    expect(effortOptions(unsent)[0]).toEqual({ value: UNASKED, label: 'Default' });
  });

  it('reads the thread’s latest choice, Default as none', () => {
    expect(chosenEffort([])).toBeNull();
    expect(
      chosenEffort([
        event(1, 'effort.chosen', { effort: 'high' }),
        event(2, 'effort.chosen', { effort: 'low' }),
      ]),
    ).toBe('low');
    expect(
      chosenEffort([event(1, 'effort.chosen', { effort: 'high' }), event(2, 'effort.chosen', {})]),
    ).toBeNull();
  });

  it('sends a pick only when the model offers it and it changes the thread’s choice', () => {
    expect(effortToSend('high', null, REASONS)).toBe('high');
    expect(effortToSend('high', 'high', REASONS)).toBeUndefined();
    expect(effortToSend('max', null, REASONS)).toBeUndefined();
    expect(effortToSend(null, 'low', REASONS)).toBeUndefined();
  });

  it('sends Default as null to go back from a level, and nothing where there is none', () => {
    expect(effortToSend(UNASKED, 'high', REASONS)).toBeNull();
    // Whatever the model offers: from a level it doesn't take, Default is the way back.
    expect(effortToSend(UNASKED, 'max', undefined)).toBeNull();
    expect(effortToSend(UNASKED, null, REASONS)).toBeUndefined();
  });
});
