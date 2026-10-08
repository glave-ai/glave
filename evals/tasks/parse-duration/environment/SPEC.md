# Durations

`parse_duration` turns text like `1h30m` into a number of seconds.

- A duration is one or more parts. Each part is a whole number followed by a unit:
  `d` (days), `h` (hours), `m` (minutes) or `s` (seconds).
- Units appear in that order, each at most once: `1h30m` is valid; `30m1h` and `1h1h` are not.
- Spaces (` `) may separate parts: `1h 30m`. A space inside a part (`1 h`) is not allowed.
  Spaces before the first part and after the last are ignored.
- Numbers may have leading zeros (`05m`). Units are lowercase only.

Errors:

- `Empty`: the text is empty or only spaces.
- `Overflow`: a number, or the total in seconds, does not fit in a `u64`.
- `Invalid`: anything else that breaks these rules.
