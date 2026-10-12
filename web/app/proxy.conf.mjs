// `npm start`: the dev server serves the app; the session API's requests (POST /) go to a
// `glave serve --http` on this computer, at GLAVE_URL or http://127.0.0.1:8080.
const target = process.env['GLAVE_URL'] ?? 'http://127.0.0.1:8080';

export default {
  '/': {
    target,
    secure: false,
    // Pages and assets (GET) stay with the dev server; only API calls are forwarded.
    bypass: (request) => (request.method === 'POST' ? undefined : request.url),
  },
};
