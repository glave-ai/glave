import { ApiError, ErrorCode } from '@glave/client';

import { event } from '../testing/fixtures';
import { alreadyEnded, commandProblem, lastCommandChange } from './commands';
import { somethingWrong } from './text';

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
    expect(commandProblem(new TypeError('Failed to fetch'))).toBe(somethingWrong());
  });
});
