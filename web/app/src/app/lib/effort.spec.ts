import type { ModelInfo } from '@glave/client';

import { event } from '../testing/fixtures';
import { chosenEffort, defaultEffort, effortOptions, effortToSend, levelName } from './effort';

const REASONS: ModelInfo = {
  id: 'openai/gpt-6-luna',
  default: true,
  reasoning: { efforts: ['xhigh', 'high', 'medium', 'low', 'none'], default: 'medium' },
};

describe('effort', () => {
  it('offers exactly the levels the model lists, its default marked', () => {
    expect(effortOptions(REASONS)).toEqual([
      { value: 'xhigh', label: 'Extra high' },
      { value: 'high', label: 'High' },
      { value: 'medium', label: 'Medium', hint: 'Default' },
      { value: 'low', label: 'Low' },
      { value: 'none', label: 'Off' },
    ]);
    expect(effortOptions({ id: 'm', default: false })).toEqual([]);
    expect(effortOptions({ id: 'm', default: false, reasoning: { mandatory: true } })).toEqual([]);
    expect(effortOptions(undefined)).toEqual([]);
    expect(defaultEffort(REASONS)).toBe('medium');
    expect(levelName('turbo')).toBe('Turbo');
  });

  it('marks no level where none is sent unasked, which is the model maker’s default', () => {
    const unsent: ModelInfo = { id: 'm', default: false, reasoning: { efforts: ['high', 'low'] } };
    expect(effortOptions(unsent).some((option) => option.hint !== undefined)).toBe(false);
    expect(defaultEffort(unsent)).toBeNull();
  });

  it('reads the thread’s latest choice', () => {
    expect(chosenEffort([])).toBeNull();
    expect(
      chosenEffort([
        event(1, 'effort.chosen', { effort: 'high' }),
        event(2, 'effort.chosen', { effort: 'low' }),
      ]),
    ).toBe('low');
  });

  it('sends a pick only when the model offers it and it changes the thread’s choice', () => {
    expect(effortToSend('high', null, REASONS)).toBe('high');
    expect(effortToSend('high', 'high', REASONS)).toBeNull();
    expect(effortToSend('max', null, REASONS)).toBeNull();
    expect(effortToSend(null, 'low', REASONS)).toBeNull();
  });
});
