import { pageFromUrl, threadFromUrl, threadMark, threadTitle } from './threads';

describe('threads', () => {
  it('reads the thread from a URL', () => {
    expect(threadFromUrl('/t/abc')).toBe('abc');
    expect(threadFromUrl('/t/abc?x=1')).toBe('abc');
    expect(threadFromUrl('/tasks')).toBeNull();
  });

  it('reads the page from a URL', () => {
    expect(pageFromUrl('/tasks')).toBe('tasks');
    expect(pageFromUrl('/memory?x=1')).toBe('memory');
    expect(pageFromUrl('/settings#a')).toBe('settings');
    expect(pageFromUrl('/tasksforce')).toBeNull();
    expect(pageFromUrl('/t/abc')).toBeNull();
  });

  it('marks running and waiting threads', () => {
    expect(threadMark({ status: 'running' })).toEqual({ text: 'running', tone: 'neutral' });
    expect(threadMark({ status: 'waiting' })).toEqual({ text: 'needs you', tone: 'accent' });
    expect(threadMark({ status: 'idle' })).toBeNull();
  });

  it('titles a thread, or calls it new', () => {
    expect(threadTitle({ title: 'Plan' })).toBe('Plan');
    expect(threadTitle({ title: '  ' })).toBe('New chat');
    expect(threadTitle({ title: null })).toBe('New chat');
  });
});
