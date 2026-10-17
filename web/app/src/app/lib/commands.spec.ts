import { ApiError, ErrorCode } from '@glave/client';

import { event } from '../testing/fixtures';
import { alreadyEnded, commandProblem, lastCommandChange, runningFor } from './commands';
import { SOMETHING_WRONG } from './text';

describe('lastCommandChange', () => {
  it('is the latest start or end of a command in the background', () => {
    expect(lastCommandChange([])).toBeNull();
    expect(lastCommandChange([event(1, 'message.added', {})])).toBeNull();
    expect(
      lastCommandChange([
        event(1, 'command.started', { command: 'c1' }),
        event(2, 'tool.call.completed', {}),
        event(3, 'command.ended', { command: 'c1' }),
        event(4, 'message.added', {}),
      ]),
    ).toBe(3);
  });
});

describe('runningFor', () => {
  const start = '2026-10-11T10:00:00.000Z';
  const at = (offset: number): number => Date.parse(start) + offset;

  it('is how long it has run, in whole seconds, and at least one', () => {
    expect(runningFor(start, at(125_400))).toBe(125_000);
    expect(runningFor(start, at(999))).toBe(1000);
    // A clock behind the server's still reads a second.
    expect(runningFor(start, at(-5000))).toBe(1000);
  });

  it('is unknown until the start is recorded, or when it does not parse', () => {
    expect(runningFor(null, at(0))).toBeNull();
    expect(runningFor(undefined, at(0))).toBeNull();
    expect(runningFor('soon', at(0))).toBeNull();
  });
});

describe('alreadyEnded', () => {
  it('is the refusal for a command no longer running', () => {
    expect(alreadyEnded(new ApiError({ code: ErrorCode.NO_COMMAND, message: 'gone' }))).toBe(true);
    expect(alreadyEnded(new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'no' }))).toBe(false);
    expect(alreadyEnded(new Error('offline'))).toBe(false);
  });
});

describe('commandProblem', () => {
  it("says the server's own words, or the plain failure", () => {
    expect(
      commandProblem(
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the command could not be stopped' }),
      ),
    ).toBe('The command could not be stopped.');
    expect(commandProblem(new TypeError('Failed to fetch'))).toBe(SOMETHING_WRONG);
  });
});
