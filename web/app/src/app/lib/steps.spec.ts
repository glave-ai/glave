import { SquareTerminal, Wrench } from 'lucide';

import { toolIcon } from './steps';

describe('steps', () => {
  it('gives the harness’s tools their own icons, and any other a wrench', () => {
    expect(toolIcon('bash')).toBe(SquareTerminal);
    expect(toolIcon('mcp__github__search')).toBe(Wrench);
  });
});
