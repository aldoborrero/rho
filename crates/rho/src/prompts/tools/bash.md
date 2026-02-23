Executes bash commands in a shell session for terminal operations like git, cargo, npm, docker.

- Use `cwd` parameter to set working directory instead of `cd dir && ...`
- Paths with spaces must use double quotes: `cd "/path/with spaces"`
- For sequential dependent operations, chain with `&&`: `mkdir foo && cd foo && touch bar`
- For parallel independent operations, make multiple tool calls in one message
- Use `;` only when later commands should run regardless of earlier failures

Output: stdout and stderr merged, exit code on non-zero. Truncated after 100KB.

Do NOT use Bash for these operations—specialized tools exist:
- Reading file contents -> Read tool
- Searching file contents -> Grep tool
- Finding files by pattern -> Find tool
- Writing new files -> Write tool
