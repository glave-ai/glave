import { event } from '../testing/fixtures';
import { lastPlanning } from './plans';

describe('lastPlanning', () => {
  it('is the seq of the latest change to the plan, the model’s, the person’s or Build it', () => {
    expect(lastPlanning([])).toBeNull();
    expect(lastPlanning([event(1, 'turn.started', {}), event(2, 'turn.ended', {})])).toBeNull();
    const proposed = event(3, 'plan.updated', { steps: [], proposed: true });
    expect(lastPlanning([event(1, 'turn.started', {}), proposed, event(4, 'turn.ended', {})])).toBe(
      3,
    );
    const built = [proposed, event(5, 'plan.approved', {}), event(6, 'turn.started', {})];
    expect(lastPlanning(built)).toBe(5);
  });
});
