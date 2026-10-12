import type { ModelInfo } from '@glave/client';

import { event } from '../testing/fixtures';
import {
  chosenModel,
  companyName,
  defaultModel,
  displayName,
  modelOptions,
  slugWords,
  toPinned,
} from './models';

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

  it('names models Company · Model, from the name or the id, never as a slug', () => {
    expect(displayName({ id: 'anthropic/claude-haiku-5.5' })).toBe('Anthropic · Claude Haiku 5.5');
    expect(displayName({ id: 'openai/gpt-6-luna' })).toBe('OpenAI · GPT-6 Luna');
    expect(displayName({ id: 'deepseek/deepseek-v4.1-flash' })).toBe(
      'DeepSeek · DeepSeek V4.1 Flash',
    );
    expect(displayName({ id: 'anthropic/claude-haiku-5.5', name: 'Claude Haiku 5.5' })).toBe(
      'Anthropic · Claude Haiku 5.5',
    );
    expect(displayName({ id: 'x', name: 'Anthropic: Claude Haiku 5.5' })).toBe(
      'Anthropic · Claude Haiku 5.5',
    );
    expect(displayName({ id: 'acme-labs/rocket-2:free' })).toBe('Acme Labs · Rocket 2 (Free)');
    expect(displayName({ id: 'openrouter/meta-llama/llama-4-scout' })).toBe('Meta · Llama 4 Scout');
    expect(displayName({ id: 'hope-12b', name: 'Hope' })).toBe('Hope');
    expect(displayName({ id: 'gpt-6-luna', name: '  ' })).toBe('GPT-6 Luna');
  });

  it('writes slugs and companies as their makers do', () => {
    expect(slugWords('gpt-6')).toBe('GPT-6');
    expect(slugWords('gpt-oss-120b')).toBe('GPT OSS 120b');
    expect(slugWords('qwen3_vl')).toBe('Qwen3 VL');
    expect(companyName('x-ai')).toBe('xAI');
    expect(companyName('MoonshotAI')).toBe('Moonshot AI');
    expect(companyName('some-lab')).toBe('Some Lab');
  });

  it('offers each model by its display name, marking the default and Hope only by its flag', () => {
    expect(modelOptions(MODELS)).toEqual([
      { value: 'gpt-6-luna', label: 'GPT-6 Luna', hope: false, hint: 'Default' },
      { value: 'hope-12b', label: 'Hope', hope: true },
      { value: 'hope-lookalike', label: 'Hope Lookalike', hope: false },
    ]);
  });

  it('reads back the pinned models’ ids only', () => {
    expect(toPinned(['a', 3, '', 'b'])).toEqual(['a', 'b']);
    expect(toPinned('a')).toEqual([]);
    expect(toPinned(null)).toEqual([]);
  });
});
