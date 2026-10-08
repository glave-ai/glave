# tally

`tally` counts lines, words and bytes, like a small `wc`.

    tally [-l] [-w] [-c] [FILE...]

- `-l` counts lines, `-w` words and `-c` bytes. With none of them, all three. Flags may come
  in any order and be combined (`-lw`); counts always print in the order lines, words, bytes.
- Lines are `\n` characters. Words are runs of characters that are not ASCII whitespace.
  Bytes are bytes.
- For each file, one line: the chosen counts, then the file's name, separated by single
  spaces: `3 12 80 notes.txt`.
- With more than one file named, a last line totals them, named `total`.
- With no files, it reads standard input and prints the counts with no name.
- A file that cannot be read prints `tally: NAME: cannot read` to standard error, is left out
  of the total, and makes the exit status 1; the other files are still counted.
- An unknown flag prints `tally: unknown flag -x` (naming the flag's letter) to standard
  error and exits with status 2, counting nothing.
