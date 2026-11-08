import { tapped } from './tapped';

const THREAD = '019a0000-0000-7000-8000-000000000001';
const ORG = '019a0000-0000-7000-8000-0000000000aa';

describe('tapped', () => {
  it('is the thread and organization the worker asks the app to open', () => {
    const page = `/t/${THREAD}`;
    expect(tapped({ glave: 'open', page, org: ORG })).toEqual({ page, org: ORG });
    expect(tapped({ glave: 'open', page: '/', org: null })).toEqual({ page: '/', org: null });
    expect(tapped({ glave: 'open', page, org: 'not an id' })).toEqual({ page, org: null });
  });

  it('is nothing for any other message, or a page that is no thread’s', () => {
    for (const message of [
      null,
      'open',
      { page: `/t/${THREAD}` },
      { glave: 'open', page: '/settings' },
      { glave: 'open', page: 'https://example.com/' },
      { glave: 'open', page: `/t/${THREAD}/../../admin` },
    ]) {
      expect(tapped(message)).toBeNull();
    }
  });
});
