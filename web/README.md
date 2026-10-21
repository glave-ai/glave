# glave on the web

Four parts, one npm project:

| Folder    | What it is                                                                                                                                                      |
| --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `client/` | The session API's TypeScript client. Its types are generated from `specs/session.schema.json` by `cargo xtask schema`.                                          |
| `glaze/`  | **Glaze** (`@glave/glaze`), glave's Angular component library, and its showcase. See [glaze/README.md](glaze/README.md) and [glaze/AGENTS.md](glaze/AGENTS.md). |
| `app/`    | The web app `glave serve --http` serves: sign-in, projects and threads, a live thread, tasks, memory, settings.                                                 |
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

A release carries the build inside the `glave` binary, which `glave serve --http` (and so `glave` alone) serves when no `--web` is given: `npm run build`, then `cargo build --release -p glave --features web`. Without the feature, glave needs no Node to build and serves a `web` folder beside the binary, if there is one.

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
npm run e2e            # Playwright in Chromium and WebKit against glave serve --http --web, axe in light and dark
```

`cargo xtask ci` runs `npm run check`, after building the app and `glave` with it inside (and its serving tests) for the end-to-end tests, which run against the real `glave serve --http --web`. It needs Node 22.22.3 or later; `GLAVE_SKIP_WEB=1` skips it. Run `npm run e2e` by itself after `cargo build -p glave` (or set `GLAVE_BIN`).

Every glave the end-to-end tests start listens on a port the system picks, so two runs on one machine at once (two `cargo xtask ci` side by side) never meet. `npm run e2e:twice` shows it: it runs the tests twice at once and fails unless both pass, taking Playwright's arguments after `--` (such as `-- --project=chromium`). It is not part of `npm run check`, since it doubles the tests' time.

Playwright's Chromium and WebKit (Safari's engine) are installed into `node_modules` (`npm run e2e:install`, `PLAYWRIGHT_BROWSERS_PATH=0`), nothing global. Every end-to-end test runs in both, headless; one that skips WebKit says why.

## How the app is built

- **Three panes**, as in Claude's desktop app: the side nav (New chat, a search that finds threads by title or by day, Needs you pinned at the top while something waits for the person, the projects then the loose chats, each list newest first in one order with no sort to choose, each thread with, at its end, a ring that turns while it runs (still under reduced motion), a small mark while it waits for the person, else how long ago it was last active, each heading closing its list (a closed one keeps the mark when something in it waits), with + to start a project, a thread in the project or a chat there and a project's ⋯ to rename it, a thread's subagents indented under it, the Tasks, Memory and Settings pages, and the person signed in); the page, under a top bar that floats over it and fades into it (a thread's bar is ruled off instead: its title, which renames it, its project and where it runs, and "Changes" with how many files it changed); and, in a thread, the tools panel (its Tasks tab, the background tasks; its Changes tab, the files the thread changed with their diffs; and its Commands tab, the commands the thread keeps running in the background, each with how long it has run and Stop), closed at first.
- **The side panels have no toggle buttons on wide screens.** Each one's edge is a window splitter (WAI-ARIA): drag it to resize, past half its minimum to collapse; a collapsed panel leaves a slim grip at the side of the page to drag or click back. From the keyboard, Tab to the edge, then the arrow keys resize, Home and End go to the bounds, and Enter collapses and restores. Each panel's width and state are remembered in this browser (local storage, when it allows). On narrow screens the side nav is a drawer and the tools a sheet, each opened from a button in the top bar.
- **Keyboard shortcuts**, also in the edges' tooltips:

  | Shortcut              | Does                                  |
  | --------------------- | ------------------------------------- |
  | ⌘B (Ctrl+B elsewhere) | Shows or hides the side nav           |
  | ⌘. (Ctrl+. elsewhere) | Shows or hides a thread's tools panel |
  | Enter, Shift+Enter    | In the composer: sends, or a new line |

- **Installable.** glave serves a web app manifest (`manifest.webmanifest`, `display: standalone`, its colours Glaze's page surface) and icons, which the Content Security Policy allows (`manifest-src 'self'`). Chromium needs no service worker to install it, so there is none. The app shows no install button of its own: browsers offer it from their own menus.
- **The theme** follows the system until the person picks Light or Dark in Settings, the one place to choose it.
- `services/`: `SessionService` (the token and every call), `ProjectsService` (projects and their threads: read, then followed live with `thread/list/watch` while the tab is shown, each thread's summary taken as the server says it changed, read again only when the watch comes back after being cut off or the tab is shown again, and after the app changes a project; what follows them, `lists-feed.ts`, loads after the first page), `ThreadService` (one open thread: its events, the reply streaming in, decisions, answers, the model it chose, renaming; each thread view has its own), `CommandsService` (the open thread's commands in the background, read again when the thread records one starting or ending and when their tab opens, and stopping one; beside each `ThreadService`), `ModelsService` (`model/list`), `RecentModelsService` (the models the person picked lately, kept in local storage when the browser allows), `ServerService` (`server/discover`: the server's approval mode and the modes a thread may choose), `TasksService` (read again every few seconds while a list shows), `MemoryService`, `NavService` (the side nav open or collapsed and its width, kept in local storage when the browser allows; its shortcut), `NavListsService` (the side nav's lists closed, kept the same way), `ToolsPanelService` (the tools panel open, its width and tab, kept the same way; its shortcut), `ThemeService` (the person's System, Light or Dark choice, until settings live on the server).
- `lib/`: pure functions: the thread built from its events (`conversation.ts`), split into turns with what each changed, took and cost (`turns.ts`) and shown results first (`results.ts`), each tool's icon (`steps.ts`), what an approval asks in a sentence (`approvals.ts`), where a thread runs (`place.ts`), the files a thread changed (`files.ts`, from its successful `write` and `edit` calls), the side nav's lists, ordered, grouped and searched (`nav.ts`), as the person left them (`nav-lists.ts`) and as the server's summaries change them (`lists.ts`), when a watch cut off comes back (`watching.ts`), the server's times as the reader reads them, how long ago and the words a day is found by, in their language and time zone (`when.ts`), models (each named `Company · Model` from its name or id, never a slug, and ordered for the picker: `models.ts`), reasoning levels as the model lists them (`effort.ts`), the language the browser asks for (`language.ts`), a message's plurals in the reader's language (`plural.ts`), Glaze's words as the app's messages, for a translation (`glaze-text.ts`), when a message's text is long enough to fold, the panels' stored state, storage that may refuse, the shortcuts and how they read on each platform, the token from the link, thread titles.
- `components/`: one per page or part (workspace, side nav, projects nav, nav thread, nav list menu, person, page header, home, thread, thread header, turn, thread item, turn changes, message text (a reply as markdown; a long message of the person's folds, with Show more), model picker, effort picker, approval mode, tools panel, task list, thread files, command list, rename dialog, tasks, memory, settings, awake (in Settings, on the person's own computer: whether it stays awake while glave works, as the server says), sign-in).
- Messages, as in a text conversation: the person's own are bubbles at the end of the line, a long one folded behind Show more; replies are full-width markdown (Glaze's `glzMarkdown`: tables, task lists, code with a copy button and highlighting; images only as links, never fetched), streamed block by block, with no model's name over them. No avatars. Under each message, a small copy button (its text as written) shows on hover and on keyboard focus, always on touch, and says "Copied", announced to screen readers.
- **Results first.** A turn that did work shows, after the person's message, its reply; "What changed", a card with how many files and lines added and removed, and a row per file that opens the tools panel's changes; then its receipt, one sunken line with how it went, how long it took and what it cost, which opens to the steps. The steps are the turn's work in order: tool calls as quiet lines (the tool's icon, its name and what it acted on), what it said along the way, and the questions and approvals already answered. What waits for the person comes last, never folded: a "Needs you" card with one plain sentence, what would run in monospace and every answer, the primary first. A turn that did no work is its messages alone. Every box spans the reading column.
- The composer's box holds only what the person writes: the text, where it goes (the home's project chip) and Send, which becomes Stop while a turn runs. Under it a quiet line holds the approval mode (its icon and word: Ask or Auto, among the modes the server lets a thread choose, sent with the next `thread/start` or `turn/start`; read-only when the policy leaves one), the model by its own name, and, when that model offers a choice, how much it reasons: exactly the levels its provider lists (`Extra high`, `High`, `Off`…), its default shown until the person picks, sent with the next `thread/start` or `turn/start`. On a phone the box is one row, an options button, the text and Send, and the options button opens that line as a sheet. The picker lists models `Company · Model`, each with a line saying the service that runs it, its context and its price per million tokens, as the server gives them; searches by name or service as you type (no autocorrect), scrolls a catalogue of hundreds, and lists the default first, then the models the person picked lately, then the rest. Attachments stay hidden until the API takes them.
- The sign-in token comes from the link's fragment, is taken out of the address bar before the router starts, and is kept only in the tab's session storage. It is never in a URL, a cookie or a log.

## Languages

The app is written in English, and every word it writes is a message, so another language is a file beside it. Only English ships until a speaker has read a translation; the plumbing is proven by two pseudo-locales instead, as Android and Chrome do: `en-XA` (accented, a third longer, bracketed, so text that skips the messages or a cut shows at once) and `ar-XB` (right to left). They are made by `npm run i18n` from English, never by hand, and only a browser set to one gets it (Chrome started with `--lang=en-XA`; the end-to-end tests set the browser's languages, which WebKit, as Safari, takes only from the system): they are not offered to people.

- **Messages** are Angular's own: `$localize`:@@area.key:Text``, a stable id per message (the component or `lib/` file, then a key), placeholders named (`${name}:NAME:`), plurals as ICU in the message (`{${count}:COUNT:, plural, one {# file} other {# files}}`, written by `lib/plural.ts` for the reader's language, so a translation brings the cases its language has; a case holds no placeholder). A message is read where it is used, in a function or a class field, never at a module's top level, which is read before a translation loads; lint fails on that, and on a message with no id. Glaze takes no messages: its own few words are its inputs' defaults, and a translation gives them all at once through `GLZ_TEXT` (`lib/glaze-text.ts`).
- **One build, every language inside.** Translations load at run time (`@angular/localize`'s `loadTranslations`), before the app starts (`main.ts`): English loads nothing more, and a translation's file is fetched only for the one read in it (`translate.ts`). If it can't load, the app starts in English. `@angular/localize` is a development dependency, as `ng add` makes it: what ships of it is its small runtime (MIT), and the Babel tools behind it stay in the build.
- **The language** is the browser's: the first of its languages the app has, by its whole tag or its language alone (`en-GB` reads English), else English in the region of the browser's first language (`en-DE` for `de-DE`). The page's `lang` says it, and dates, times, numbers, sizes and money follow it through `Intl` and Angular's `LOCALE_ID`.
- **Checked.** `app/src/locale/` holds English as Angular extracts it (`messages.json`) and each translation (`messages.<tag>.json`), written by `npm run i18n`. `npm run build` fails when a translation lacks a message, has one English does not, differs from it in placeholders, or has a plural that does not close, has no `other` or holds a placeholder, or when a pseudo-locale is not what the script makes; `npm run i18n:check`, in `npm run check`, fails too when the source's messages changed and English was not extracted again. The end-to-end tests read the app in `en-XA` and fail on any plain English the server did not write.
- **A real language** joins with its file, its tag in `TRANSLATED` (`lib/language.ts`) and its loader in `translate.ts`, once a speaker has read it.

## Rules

`AGENTS.md` at the repository root, "Web client", holds the rules: four files per component, folders by kind, no logic in templates, `data-testid` hooks for end-to-end tests (`<area>-<part>`, such as `composer-send`), and strict lint and tests.
