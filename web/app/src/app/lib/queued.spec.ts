import { event } from '../testing/fixtures';
import { named, notRead, together } from './queued';

describe('queued', () => {
  const waiting = [
    { key: 'q1', text: 'actually, 4000', id: null },
    { key: 'q2', text: 'and keep the comment', id: null },
  ];

  it('knows a message by the id the server answered with', () => {
    expect(named(waiting, 'q2', 'event-9')).toEqual([
      { key: 'q1', text: 'actually, 4000', id: null },
      { key: 'q2', text: 'and keep the comment', id: 'event-9' },
    ]);
  });

  it('leaves out what the model has read: an event records it', () => {
    const known = named(named(waiting, 'q1', 'event-3'), 'q2', 'event-4');
    const events = [event(3, 'message.added', { role: 'user', parts: [] })];
    expect(notRead(known, events)).toEqual([
      { key: 'q2', text: 'and keep the comment', id: 'event-4' },
    ]);
    expect(notRead(waiting, events)).toEqual(waiting);
  });

  it('sends what was never read as one message, in order', () => {
    expect(together(['actually, 4000', 'and keep the comment'])).toBe(
      'actually, 4000\n\nand keep the comment',
    );
  });
});
