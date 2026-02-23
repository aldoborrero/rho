Search file contents using ripgrep.

- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`)
- Filter files with `glob` (e.g., `*.js`, `**/*.tsx`) or `type` (e.g., `js`, `py`, `rust`)
- Pattern syntax uses ripgrep—literal braces need escaping (`interface\{\}` to find `interface{}` in Go)
- For cross-line patterns, set `multiline: true`
- Results truncated at 100 matches by default (configurable via `limit`)

ALWAYS use Grep for search tasks—NEVER invoke `grep` or `rg` via Bash.
