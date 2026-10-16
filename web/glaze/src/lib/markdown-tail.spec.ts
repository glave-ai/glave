import { settleTail } from './markdown-tail';

describe('settleTail', () => {
  it.each([
    // Strong, emphasis and strikethrough close as they are written.
    ['**bold', '**bold**'],
    ['Some **bold and *italic', 'Some **bold and *italic***'],
    ['*it', '*it*'],
    ['_it', '_it_'],
    ['__strong', '__strong__'],
    ['~~old', '~~old~~'],
    ['~~old~', '~~old~~'],
    ['**bold*', '**bold**'],
    ['About ~5 minutes', 'About ~5 minutes'],
    ['**done** and **half', '**done** and **half**'],
    // An opener with nothing after it yet goes.
    ['Some **', 'Some'],
    ['Some *', 'Some'],
    ['Wait `', 'Wait'],
    // Code spans close, and nothing inside them counts.
    ['Run `cargo **test', 'Run `cargo **test`'],
    ['A ``tick ` inside', 'A ``tick ` inside``'],
    // Links show their text until the address is whole; images show nothing yet.
    ['See [the docs](https://exa', 'See the docs'],
    ['See [**the** docs](https://exa', 'See **the** docs'],
    ['A chart ![sales](https://evil.example/?q=', 'A chart'],
    ['[done](https://a.example) and [half](htt', '[done](https://a.example) and half'],
    // What is not markup stays as it is.
    ['snake_case_name', 'snake_case_name'],
    ['2 * 3 = 6', '2 * 3 = 6'],
    ['* item', '* item'],
    ['Escaped \\*star', 'Escaped \\*star'],
    ['Done.', 'Done.'],
  ])('settles the last line %j as %j', (raw, settled) => {
    expect(settleTail(raw, true)).toBe(settled);
  });

  it.each([
    // A line that may yet start a list, a heading, a quote or a rule, or turn the paragraph
    // above into a heading, waits for what follows.
    ['Para\n-', 'Para'],
    ['Para\n=', 'Para'],
    ['Para\n---', 'Para'],
    ['- a\n- ', '- a'],
    ['1. a\n2.', '1. a'],
    ['1. a\n2', '1. a'],
    ['- [x] a\n- [', '- [x] a'],
    ['- [x] a\n- [ ]', '- [x] a'],
    ['Text\n#', 'Text'],
    ['Text\n> ', 'Text'],
    ['Text\n``', 'Text'],
  ])('holds back the undecided last line of %j', (raw, settled) => {
    expect(settleTail(raw, true)).toBe(settled);
  });

  it('holds back table rows until their delimiter row makes them a table', () => {
    expect(settleTail('Totals:\n| a | b |\n| -', true)).toBe('Totals:');
    expect(settleTail('| a | b |', true)).toBe('');
    // Once marked reads a table, its rows show as they come.
    expect(settleTail('| a | b |\n| - | - |\n| **1', false)).toBe('| a | b |\n| - | - |\n| **1**');
  });

  it('leaves code being written alone', () => {
    expect(settleTail('```rust\nlet a = **b', true)).toBe('```rust\nlet a = **b');
    expect(settleTail('1. Run\n   ```sh\n   echo *', true)).toBe('1. Run\n   ```sh\n   echo *');
    expect(settleTail('```ru', true)).toBe('```ru');
  });
});
