import { built, promptOf, sent, withFiles } from './starts';

describe('starts', () => {
  it('starts a turn with the person’s message, or with a plan to build', () => {
    const message = sent({ thread: 'thread-1', prompt: 'Fix it' });
    expect(message).toEqual({
      method: 'turn/start',
      params: { thread: 'thread-1', prompt: 'Fix it' },
    });
    expect(promptOf(message)).toBe('Fix it');
    const build = built('thread-1');
    expect(build).toEqual({ method: 'plan/build', params: { thread: 'thread-1' } });
    expect(promptOf(build)).toBeNull();
  });

  it('sends files only when there are some', () => {
    expect(withFiles([])).toEqual({});
    const file = { sha256: 'abc', name: 'a.txt' };
    expect(withFiles([file])).toEqual({ files: [file] });
  });
});
