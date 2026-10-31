import { type GlzServiceField, answerOf, sameFields, startingValues } from './service-fields';

const FIELDS: readonly GlzServiceField[] = [
  { name: 'words', label: 'Words', required: true, kind: { type: 'text', default: 'hi' } },
  { name: 'count', label: 'Count', kind: { type: 'number', integer: true, default: 2 } },
  { name: 'part', label: 'Part', kind: { type: 'number' } },
  { name: 'sure', label: 'Sure', kind: { type: 'yes_no', default: false } },
  {
    name: 'one',
    label: 'One',
    kind: { type: 'choice', options: [{ value: 'a', label: 'A' }], default: ['a'] },
  },
  {
    name: 'many',
    label: 'Many',
    kind: { type: 'choice', options: [{ value: 'b', label: 'B' }], multiple: true },
  },
];

describe('service fields', () => {
  it('start as the service says', () => {
    expect(startingValues(FIELDS)).toEqual({
      words: 'hi',
      count: '2',
      part: undefined,
      sure: false,
      one: 'a',
      many: [],
    });
  });

  it('answer only what was given, each as its kind reads', () => {
    expect(answerOf(FIELDS, startingValues(FIELDS))).toEqual({
      words: 'hi',
      count: 2,
      sure: false,
      one: 'a',
    });
    expect(answerOf(FIELDS, { words: ' x ', part: '1.5', many: ['b'] })).toEqual({
      words: 'x',
      part: 1.5,
      many: ['b'],
    });
  });

  it('answer nothing while a required one is empty or a number cannot be read', () => {
    expect(answerOf(FIELDS, { words: '  ' })).toBeNull();
    expect(answerOf(FIELDS, { words: 'x', count: '1.5' })).toBeNull();
    expect(answerOf(FIELDS, { words: 'x', part: 'many' })).toBeNull();
  });

  it('ask the same when their fields are the same', () => {
    expect(sameFields(FIELDS, [...FIELDS])).toBe(true);
    expect(sameFields(FIELDS, FIELDS.slice(1))).toBe(false);
  });
});
