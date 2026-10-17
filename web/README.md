# glave on the web

Four parts, one npm project:

| Folder    | What it is                                                                                                                                                      |
| --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `client/` | The session API's TypeScript client. Its types are generated from `specs/session.schema.json` by `cargo xtask schema`.                                          |
| `glaze/`  | **Glaze** (`@glave/glaze`), glave's Angular component library, and its showcase. See [glaze/README.md](glaze/README.md) and [glaze/AGENTS.md](glaze/AGENTS.md). |
| `app/`    | The web app `glave serve --http` serves: sign-in, spaces and threads, a live thread, tasks, memory, settings.                                                   |
| `e2e/`    | End-to-end smoke tests (Playwright and axe) of the production builds of the app and the showcase, served under glave's Content Security Policy.                 |

Angular 22 (standalone, zoneless, signals), TypeScript 6.0, Vitest for unit tests, Playwright for end-to-end tests. Node 22.22.3 or later (Angular's minimum).

## The dev loop

```sh
cd web
npm ci                       # once
npm start                    # the app on http://localhost:4200, rebuilt as you edit
npm run showcase             # or Glaze's showcase, every component on one page
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

A release carries the build inside the `glave` binary, which `glave serve --http` and `glave app` serve when no `--web` is given: `npm run build`, then `cargo build --release -p glave --features web`. Without the feature, glave needs no Node to build and serves a `web` folder beside the binary, if there is one.

## Checks

```sh
npm run check     # everything CI runs, in order:
npm run format:check   # Prettier
npm run lint           # eslint (typescript-eslint strict, angular-eslint) and stylelint
npm run typecheck      # tsc, strict
npm test               # Vitest: every component, directive, service and lib function, with axe; prints coverage
npm run build:glaze    # Glaze as a publishable Angular package (ng-packagr)
npm run build          # the app, production, within its size budget (angular.json)
npm run build:showcase # Glaze's showcase, for the end-to-end tests
npm run licenses       # every production dependency's licence is one deny.toml allows (plus OFL fonts, 0BSD)
npm run audit          # npm audit of production dependencies, high and above
npm run e2e            # Playwright in Chromium against glave serve --http --web, axe in light and dark
```

`cargo xtask ci` runs `npm run check`, after building the app and `glave` with it inside (and its serving tests) for the end-to-end tests, which run against the real `glave serve --http --web`. It needs Node 22.22.3 or later; `GLAVE_SKIP_WEB=1` skips it. Run `npm run e2e` by itself after `cargo build -p glave` (or set `GLAVE_BIN`).

Playwright's Chromium is installed into `node_modules` (`PLAYWRIGHT_BROWSERS_PATH=0`), nothing global.

## How the app is built

- **Three panes**, as in Claude's desktop app: the side nav (New chat, a search of thread titles, each project as a group of its threads that closes, with + to start one there, the loose chats, the Tasks, Memory and Settings pages, and the person signed in); the page, under a top bar that floats over it and fades into it (a thread's bar is ruled off instead: its title, which renames it, its project and where it runs, and "Changes" with how many files it changed); and, in a thread, the tools panel (its Tasks tab, the background tasks, and its Changes tab, the files the thread changed with their diffs), closed at first.
- **The side panels have no toggle buttons on wide screens.** Each one's edge is a window splitter (WAI-ARIA): drag it to resize, past half its minimum to collapse; a collapsed panel leaves a slim grip at the side of the page to drag or click back. From the keyboard, Tab to the edge, then the arrow keys resize, Home and End go to the bounds, and Enter collapses and restores. Each panel's width and state are remembered in this browser (local storage, when it allows). On narrow screens the side nav is a drawer and the tools a sheet, each opened from a button in the top bar.
- **Keyboard shortcuts**, also in the edges' tooltips:

  | Shortcut              | Does                                  |
  | --------------------- | ------------------------------------- |
  | ⌘B (Ctrl+B elsewhere) | Shows or hides the side nav           |
  | ⌘. (Ctrl+. elsewhere) | Shows or hides a thread's tools panel |
  | Enter, Shift+Enter    | In the composer: sends, or a new line |

- **Installable.** glave serves a web app manifest (`manifest.webmanifest`, `display: standalone`, its colours Glaze's page surface) and icons, which the Content Security Policy allows (`manifest-src 'self'`). Chromium needs no service worker to install it, so there is none. The app shows no install button of its own: browsers offer it from their own menus.
- **The theme** follows the system until the person picks Light or Dark in Settings, the one place to choose it.
- `services/`: `SessionService` (the token and every call), `SpacesService` (spaces and their threads, read again every ten seconds), `ThreadService` (one open thread: its events, the reply streaming in, decisions, answers, the model it chose, renaming; each thread view has its own), `ModelsService` (`model/list`), `RecentModelsService` (the models the person picked lately, kept in local storage when the browser allows), `ServerService` (`server/discover`: the server's approval mode and the modes a thread may choose), `TasksService` (read again every few seconds while a list shows), `MemoryService`, `NavService` (the side nav open or collapsed and its width, kept in local storage when the browser allows; its shortcut), `ToolsPanelService` (the tools panel open, its width and tab, kept the same way; its shortcut), `ThemeService` (the person's System, Light or Dark choice, until settings live on the server).
- `lib/`: pure functions: the thread built from its events (`conversation.ts`), split into turns with what each changed, took and cost (`turns.ts`) and shown results first (`results.ts`), each tool's icon (`steps.ts`), what an approval asks in a sentence (`approvals.ts`), where a thread runs (`place.ts`), the files a thread changed (`files.ts`, from its successful `write` and `edit` calls), the side nav's groups (`nav.ts`), models (each named `Company · Model` from its name or id, never a slug, and ordered for the picker: `models.ts`), when a message's text is long enough to fold, the panels' stored state, storage that may refuse, the shortcuts and how they read on each platform, the token from the link, thread titles and marks.
- `components/`: one per page or part (workspace, side nav, spaces nav, person, page header, home, thread, thread header, turn, thread item, turn changes, message text (a reply as markdown; a long message of the person's folds, with Show more), model picker, approval mode, tools panel, task list, thread files, rename dialog, tasks, memory, settings, sign-in).
- Messages, as in a text conversation: the person's own are bubbles at the end of the line, a long one folded behind Show more; replies are full-width markdown (Glaze's `glzMarkdown`: tables, task lists, code with a copy button and highlighting; images only as links, never fetched), streamed block by block, with no model's name over them. No avatars. Under each message, a small copy button (its text as written) shows on hover and on keyboard focus, always on touch, and says "Copied", announced to screen readers.
- **Results first.** A turn that did work shows, after the person's message, its reply; "What changed", a card with how many files and lines added and removed, and a row per file that opens the tools panel's changes; then its receipt, one sunken line with how it went, how long it took and what it cost, which opens to the steps. The steps are the turn's work in order: tool calls as quiet lines (the tool's icon, its name and what it acted on), what it said along the way, and the questions and approvals already answered. What waits for the person comes last, never folded: a "Needs you" card with one plain sentence, what would run in monospace and every answer, the primary first. A turn that did no work is its messages alone. Every box spans the reading column.
- The composer's box holds only what the person writes: the text, where it goes (the home's project chip) and Send, which becomes Stop while a turn runs. Under it a quiet line holds the approval mode (its icon and word: Ask or Auto, among the modes the server lets a thread choose, sent with the next `thread/start` or `turn/start`; read-only when the policy leaves one) and the model by its own name. On a phone the box is one row, an options button, the text and Send, and the options button opens that line as a sheet. The picker lists models `Company · Model`, searches by name as you type (no autocorrect), scrolls a catalogue of hundreds, and lists the default first, then the models the person picked lately, then the rest. Attachments stay hidden until the API takes them.
- The sign-in token comes from the link's fragment, is taken out of the address bar before the router starts, and is kept only in the tab's session storage. It is never in a URL, a cookie or a log.

## Rules

`AGENTS.md` at the repository root, "Web client", holds the rules: four files per component, folders by kind, no logic in templates, `data-testid` hooks for end-to-end tests (`<area>-<part>`, such as `composer-send`), and strict lint and tests.
