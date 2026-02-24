# Grep

<instruction>
Search file contents using ripgrep.

- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`)
- Filter files with `glob` (e.g., `*.js`, `**/*.tsx`) or `type` (e.g., `js`, `py`, `rust`)
- Pattern syntax uses ripgrep — literal braces need escaping (`interface\{\}` to find `interface{}` in Go)
- Patterns containing `\n` default to multiline mode automatically
- For other cross-line patterns, set `multiline: true`
- Results truncated at 100 matches by default (configurable via `limit`)
</instruction>

<output>
Matching lines with file paths and line numbers. Results are truncated at the configured limit.
</output>

<critical>
ALWAYS use Grep for search tasks — NEVER invoke `grep` or `rg` via Bash.
</critical>

<avoid>
Piping grep output through other commands — use the tool's built-in parameters for filtering and limiting.
</avoid>
