import type { ModelInfo } from '@glave/client';

import { event } from '../testing/fixtures';
import { chosenModel, defaultModel, modelLabel, modelOptions } from './models';

const MODELS: readonly ModelInfo[] = [
  { id: 'gpt-6-luna', default: true, hope: false },
  { id: 'hope-12b', name: 'Hope', default: false, hope: true },
  { id: 'hope-lookalike', name: '  ', default: false, hope: false },
];

describe('models', () => {
  it('reads the thread’s latest choice, or none', () => {
    expect(chosenModel([])).toBeNull();
    expect(
      chosenModel([
        event(1, 'model.chosen', { model: 'gpt-6-luna' }),
        event(2, 'message.added', { role: 'user', parts: [] }),
        event(3, 'model.chosen', { model: 'hope-12b' }),
      ]),
    ).toBe('hope-12b');
    expect(chosenModel([event(1, 'model.chosen', { model: '' })])).toBeNull();
  });

  it('finds the default, or the first, or none', () => {
    expect(defaultModel(MODELS)).toBe('gpt-6-luna');
    expect(defaultModel([{ id: 'only', default: false, hope: false }])).toBe('only');
    expect(defaultModel([])).toBeNull();
  });

  it('offers each model by name, marking the default and Hope only by its flag', () => {
    expect(modelOptions(MODELS)).toEqual([
      { value: 'gpt-6-luna', label: 'gpt-6-luna', hope: false, hint: 'Default' },
      { value: 'hope-12b', label: 'Hope', hope: true },
      { value: 'hope-lookalike', label: 'hope-lookalike', hope: false },
    ]);
  });

  it('names a reply’s model from the list, never guessing Hope', () => {
    expect(modelLabel(MODELS, 'hope-12b', 'Model')).toEqual({ name: 'Hope', hope: true });
    expect(modelLabel(MODELS, 'hope-lookalike', 'Model')).toEqual({
      name: 'hope-lookalike',
      hope: false,
    });
    expect(modelLabel(MODELS, 'hope-old', 'Model')).toEqual({ name: 'hope-old', hope: false });
    expect(modelLabel(MODELS, null, 'Model')).toEqual({ name: 'Model', hope: false });
  });
});
