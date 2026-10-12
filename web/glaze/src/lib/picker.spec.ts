import { pickerGroups } from './picker';

const OPTIONS = [
  { value: 'a', label: 'Anthropic · Claude Haiku 5.5' },
  { value: 'o', label: 'OpenAI · GPT-6 Luna' },
  { value: 'd', label: 'DeepSeek · DeepSeek V4.1 Flash' },
];

describe('pickerGroups', () => {
  it('lists everything in one group when nothing is pinned', () => {
    expect(pickerGroups(OPTIONS, [], '')).toEqual([{ kind: 'all', options: OPTIONS }]);
  });

  it('puts the pinned first, in the order they were pinned, and not again below', () => {
    const groups = pickerGroups(OPTIONS, ['d', 'a', 'gone'], '');
    expect(groups.map((group) => group.options.map((option) => option.value))).toEqual([
      ['d', 'a'],
      ['o'],
    ]);
    expect(groups.map((group) => group.kind)).toEqual(['pinned', 'all']);
  });

  it('filters both groups, and leaves out an empty one', () => {
    expect(pickerGroups(OPTIONS, ['d'], 'gpt')).toEqual([{ kind: 'all', options: [OPTIONS[1]] }]);
    expect(pickerGroups(OPTIONS, ['d'], 'flash')).toEqual([
      { kind: 'pinned', options: [OPTIONS[2]] },
    ]);
    expect(pickerGroups(OPTIONS, [], 'zebra')).toEqual([]);
  });
});
