import { askOf } from './approvals';

describe('approvals', () => {
  it('asks in a sentence for the tool, with what would run without the tool’s name', () => {
    expect(askOf('bash', 'bash: git push -u origin fix/x')).toEqual({
      sentence: 'Run this command?',
      detail: 'git push -u origin fix/x',
    });
    expect(askOf('edit', 'edit src/main.rs')).toEqual({
      sentence: 'Edit this file?',
      detail: 'src/main.rs',
    });
    expect(askOf('read', 'read .env')).toEqual({
      sentence: 'Read this file?',
      detail: '.env',
    });
    expect(askOf('github__merge', 'Merge pull request 12')).toEqual({
      sentence: 'Allow this?',
      detail: 'Merge pull request 12',
    });
  });
});
