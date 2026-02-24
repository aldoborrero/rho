# Bash

<instruction>
Executes bash commands in a shell session for terminal operations like git, cargo, npm, docker.

- Use `cwd` parameter to set working directory instead of `cd dir && ...`
- Paths with spaces must use double quotes: `cd "/path/with spaces"`
- For sequential dependent operations, chain with `&&`: `mkdir foo && cd foo && touch bar`
- For parallel independent operations, make multiple tool calls in one message
- Use `;` only when later commands should run regardless of earlier failures
</instruction>

<output>
stdout and stderr merged, exit code on non-zero. Truncated after 100KB — if you need to inspect full output, redirect to a file and read it.
</output>

<critical>
Do NOT use Bash for these operations — specialized tools exist:
- Reading file contents -> Read tool
- Editing existing files -> Edit tool
- Searching file contents -> Grep tool
- Finding files by pattern -> Find tool
- Writing new files -> Write tool
</critical>

<avoid>
- Don't pipe through `head`/`tail` for output limiting — use the specialized tool's built-in limit parameters instead.
- Don't use `2>&1` — stdout and stderr are already merged.
</avoid>
