import type { LanguageFn } from 'highlight.js';

import type { GlzHighlight } from './markdown-html';

type Loader = () => Promise<{ readonly default: LanguageFn }>;

/**
 * The languages code is highlighted in, by highlight.js's name for each, and how to load it:
 * each is its own chunk, fetched the first time a code block names it.
 */
const LOADERS: Readonly<Record<string, Loader>> = {
  bash: () => import('highlight.js/lib/languages/bash'),
  c: () => import('highlight.js/lib/languages/c'),
  cpp: () => import('highlight.js/lib/languages/cpp'),
  css: () => import('highlight.js/lib/languages/css'),
  diff: () => import('highlight.js/lib/languages/diff'),
  dockerfile: () => import('highlight.js/lib/languages/dockerfile'),
  go: () => import('highlight.js/lib/languages/go'),
  ini: () => import('highlight.js/lib/languages/ini'),
  java: () => import('highlight.js/lib/languages/java'),
  javascript: () => import('highlight.js/lib/languages/javascript'),
  json: () => import('highlight.js/lib/languages/json'),
  makefile: () => import('highlight.js/lib/languages/makefile'),
  markdown: () => import('highlight.js/lib/languages/markdown'),
  python: () => import('highlight.js/lib/languages/python'),
  rust: () => import('highlight.js/lib/languages/rust'),
  sql: () => import('highlight.js/lib/languages/sql'),
  typescript: () => import('highlight.js/lib/languages/typescript'),
  xml: () => import('highlight.js/lib/languages/xml'),
  yaml: () => import('highlight.js/lib/languages/yaml'),
};

/** Other names code blocks use for those languages. */
const ALIASES: Readonly<Record<string, string>> = {
  'c++': 'cpp',
  cc: 'cpp',
  console: 'bash',
  docker: 'dockerfile',
  golang: 'go',
  h: 'c',
  hpp: 'cpp',
  html: 'xml',
  js: 'javascript',
  jsonc: 'json',
  jsx: 'javascript',
  make: 'makefile',
  md: 'markdown',
  mjs: 'javascript',
  patch: 'diff',
  py: 'python',
  rs: 'rust',
  sh: 'bash',
  shell: 'bash',
  svg: 'xml',
  toml: 'ini',
  ts: 'typescript',
  tsx: 'typescript',
  yml: 'yaml',
  zsh: 'bash',
};

/** highlight.js's name for the language a code block names, or `null` when it has none here. */
export function highlightLanguage(named: string): string | null {
  const name = named.toLowerCase();
  const language = Object.hasOwn(ALIASES, name) ? (ALIASES[name] ?? name) : name;
  return Object.hasOwn(LOADERS, language) ? language : null;
}

/**
 * Loads highlight.js and `languages` (names from code blocks; those it lacks are skipped),
 * and gives a highlighter for every language loaded so far: highlight.js keeps them, so a
 * language is fetched once per page. `null` when none of them is one it has.
 */
export async function loadHighlighter(languages: readonly string[]): Promise<GlzHighlight | null> {
  const wanted = [...new Set(languages.map(highlightLanguage).filter((name) => name !== null))];
  if (wanted.length === 0) {
    return null;
  }
  const { default: hljs } = await import('highlight.js/lib/core');
  const loaded = await Promise.all(
    wanted.map(async (name) => ({ name, language: (await LOADERS[name]?.())?.default })),
  );
  for (const { name, language } of loaded) {
    if (language !== undefined && hljs.getLanguage(name) === undefined) {
      hljs.registerLanguage(name, language);
    }
  }
  return (code, named) => {
    const language = highlightLanguage(named);
    if (language === null || hljs.getLanguage(language) === undefined) {
      return null;
    }
    return hljs.highlight(code, { language, ignoreIllegals: true }).value;
  };
}
