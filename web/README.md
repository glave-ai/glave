# glave on the web

Four parts, one npm project:

| Folder    | What it is                                                                                                                                    |
| --------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `client/` | The session API's TypeScript client. Its types are generated from `specs/session.schema.json` by `cargo xtask schema`.                        |
| `glaze/`  | **Glaze** (`@glave/glaze`), glave's Angular component library. See [glaze/README.md](glaze/README.md) and [glaze/AGENTS.md](glaze/AGENTS.md). |
| `app/`    | The web app `glave serve --http` serves: sign-in, spaces and threads, a live thread, tasks, memory, settings; Glaze's showcase at `/glaze`.   |
| `e2e/`    | End-to-end smoke tests (Playwright and axe) of the production build, served under glave's Content Security Policy.                            |

Angular 22 (standalone, zoneless, signals), TypeScript 6.0, Vitest for unit tests, Playwright for end-to-end tests. Node 22.22.3 or later (Angular's minimum).

## The dev loop

```sh
cd web
npm ci                       # once
npm start                    # the app on http://localhost:4200, rebuilt as you edit
```

The dev server forwards the session API's calls (`POST /`) to a glave on this computer:

```sh
glave serve --http 8080      # in the project folder; prints a sign-in link
```

Point the dev server at another one with `GLAVE_URL=http://127.0.0.1:9000 npm start`.

To try the production build as glave serves it:

```sh
npm run build
glave serve --http 8080 --web web/dist/app/browser
```

A release puts the build in a `web` folder beside the `glave` binary, which `glave serve --http` serves by default.

## Checks

```sh
npm run check     # everything CI runs, in order:
npm run format:check   # Prettier
npm run lint           # eslint (typescript-eslint strict, angular-eslint) and stylelint
npm run typecheck      # tsc, strict
npm test               # Vitest: every component, directive, service and lib function, with axe
npm run build:glaze    # Glaze as a publishable Angular package (ng-packagr)
npm run build          # the app, production
npm run e2e            # Playwright in Chromium, axe in light and dark (npm run e2e:install first)
```

`cargo xtask ci` runs `npm run check` when Node is installed, and skips it with a message when it is not, or is too old. `GLAVE_SKIP_WEB=1` skips it.

Playwright's Chromium is installed into `node_modules` (`PLAYWRIGHT_BROWSERS_PATH=0`), nothing global.

## How the app is built

- `services/`: `SessionService` (the token and every call), `SpacesService` (spaces and their threads, read again every ten seconds), `ThreadService` (one open thread: its events, the reply streaming in, decisions and answers; each thread view has its own), `TasksService`, `MemoryService`, `ThemeService` (the person's light or dark choice, until settings live on the server).
- `lib/`: pure functions: the thread built from its events (`conversation.ts`), the token from the link (`token.ts`), thread titles and marks.
- `components/`: one per page or part (workspace, spaces nav, home, thread, tasks, memory, settings, sign-in, the showcase).
- The sign-in token comes from the link's fragment, is taken out of the address bar before the router starts, and is kept only in the tab's session storage. It is never in a URL, a cookie or a log.

## Rules

`AGENTS.md` at the repository root, "Web client", holds the rules: four files per component, folders by kind, no logic in templates, `data-testid` hooks for end-to-end tests (`<area>-<part>`, such as `composer-send`), and strict lint and tests.
