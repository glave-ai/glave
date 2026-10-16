import { highlightLanguage, loadHighlighter } from './highlight';

describe('highlightLanguage', () => {
  it('knows languages by their names and the other names code blocks use', () => {
    expect(highlightLanguage('rust')).toBe('rust');
    expect(highlightLanguage('RS')).toBe('rust');
    expect(highlightLanguage('ts')).toBe('typescript');
    expect(highlightLanguage('sh')).toBe('bash');
    expect(highlightLanguage('toml')).toBe('ini');
    expect(highlightLanguage('html')).toBe('xml');
  });

  it('knows nothing else, Mermaid and object keys included', () => {
    expect(highlightLanguage('mermaid')).toBeNull();
    expect(highlightLanguage('cobol')).toBeNull();
    expect(highlightLanguage('constructor')).toBeNull();
    expect(highlightLanguage('__proto__')).toBeNull();
  });
});

describe('loadHighlighter', () => {
  it('loads nothing for languages it does not have', async () => {
    await expect(loadHighlighter(['mermaid', 'cobol'])).resolves.toBeNull();
  });

  it('highlights the languages it loaded, as escaped HTML', async () => {
    const highlight = await loadHighlighter(['rs', 'mermaid']);
    const html = highlight?.('fn main() { let s = "<b>"; }', 'rust') ?? '';
    expect(html).toContain('<span class="hljs-keyword">fn</span>');
    expect(html).toContain('&lt;b&gt;');
    expect(html).not.toContain('<b>');
    expect(highlight?.('graph TD', 'mermaid')).toBeNull();
  });
});
