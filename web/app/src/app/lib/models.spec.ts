import type { ModelInfo } from '@glave/client';

import { event } from '../testing/fixtures';
import {
  chosenModel,
  companyName,
  defaultModel,
  displayName,
  modelOptions,
  orderedOptions,
  shortName,
  slugWords,
  toRecent,
  withRecent,
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

  it('names a model alone, with no company, for a button', () => {
    expect(shortName({ id: 'anthropic/claude-haiku-5.5' })).toBe('Claude Haiku 5.5');
    expect(shortName({ id: 'x', name: 'Anthropic: Claude Haiku 5.5' })).toBe('Claude Haiku 5.5');
    expect(shortName({ id: 'hope-12b', name: 'Hope' })).toBe('Hope');
  });

  it('offers each model by its display name, marking the default, Hope as any other', () => {
    expect(modelOptions(MODELS)).toEqual([
      { value: 'gpt-6-luna', label: 'GPT-6 Luna', short: 'GPT-6 Luna', hint: 'Default' },
      { value: 'hope-12b', label: 'Hope', short: 'Hope' },
      { value: 'hope-lookalike', label: 'Hope Lookalike', short: 'Hope Lookalike' },
    ]);
  });

  it('reads back the recent models’ ids only, at most five', () => {
    expect(toRecent(['a', 3, '', 'b'])).toEqual(['a', 'b']);
    expect(toRecent(['a', 'b', 'c', 'd', 'e', 'f'])).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(toRecent('a')).toEqual([]);
    expect(toRecent(null)).toEqual([]);
  });

  it('puts a model picked just now first, once, and forgets the oldest past five', () => {
    expect(withRecent([], 'a')).toEqual(['a']);
    expect(withRecent(['a', 'b'], 'b')).toEqual(['b', 'a']);
    expect(withRecent(['a', 'b', 'c', 'd', 'e'], 'f')).toEqual(['f', 'a', 'b', 'c', 'd']);
  });

  it('orders the picker: the default, the recent models latest first, then the rest', () => {
    const options = ['a', 'b', 'c', 'd', 'e'].map((value) => ({ value, label: value }));
    const order = (first: string | null, recent: readonly string[]): readonly string[] =>
      orderedOptions(options, first, recent).map((option) => option.value);
    expect(order(null, [])).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(order('c', [])).toEqual(['c', 'a', 'b', 'd', 'e']);
    expect(order('c', ['e', 'c', 'gone', 'b'])).toEqual(['c', 'e', 'b', 'a', 'd']);
  });
});
