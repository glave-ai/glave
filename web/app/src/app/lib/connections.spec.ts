import { ApiError, ErrorCode } from '@glave/client';

import { connectionProblem, serviceHost, serviceName } from './connections';

const DOCS = { url: 'https://mcp.docs.example.com/mcp', status: 'signed_out' } as const;

describe('connections', () => {
  it('names a service as the settings do, else by where it is', () => {
    expect(serviceName({ ...DOCS, name: 'docs' })).toBe('docs');
    expect(serviceName({ ...DOCS, name: '  ' })).toBe('mcp.docs.example.com');
    expect(serviceName({ ...DOCS, name: null })).toBe('mcp.docs.example.com');
    expect(serviceHost({ ...DOCS, url: 'http://127.0.0.1:8123/mcp' })).toBe('127.0.0.1:8123');
    expect(serviceHost({ ...DOCS, url: 'not an address' })).toBe('not an address');
  });

  it('tells a refusal in the server’s words, and glave’s own failures plainly', () => {
    const refused = new ApiError({
      code: ErrorCode.HOST,
      message: 'docs asks no one to sign in: it works as it is',
    });
    expect(connectionProblem(refused)).toBe('Docs asks no one to sign in: it works as it is.');
    const failed = new ApiError({ code: ErrorCode.INTERNAL, message: 'a panic at line 3' });
    expect(connectionProblem(failed)).toBe('Something went wrong. Try again in a moment.');
    expect(connectionProblem(new TypeError('Failed to fetch'))).toBe(
      'glave could not be reached. Try again.',
    );
  });
});
