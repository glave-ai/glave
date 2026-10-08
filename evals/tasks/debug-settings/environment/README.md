# settings

Reads `key = value` settings files. Blank lines and lines starting with `#` are
skipped.

Keys ignore case, and `-` and `_` are the same: `max-retries`, `Max_Retries` and
`MAX_RETRIES` all name one setting.

| key | also | default |
| --- | --- | --- |
| `timeout` | `timeout_secs` | 10 |
| `retries` | `max_retries` | 3 |
| `verbose` | | false |
