import { testIds } from './test-ids';

describe('testIds', () => {
  it('names each part after the test id, or none without one', () => {
    expect(testIds('composer', { send: 'send', removeFile: 'remove-file' })).toEqual({
      send: 'composer-send',
      removeFile: 'composer-remove-file',
    });
    expect(testIds(null, { send: 'send' })).toEqual({ send: null });
  });
});
