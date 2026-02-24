# Write

Creates or overwrites a file at the specified path.

<conditions>
- Creates parent directories if needed
- Prefer the Edit tool for modifying existing files (more precise, preserves formatting)
- Create documentation files (*.md, README) only when explicitly requested
</conditions>

<output>
Confirmation of file written with byte count.
</output>

<critical>
- No emojis unless the user explicitly requests them.
- Never write secrets, credentials, or API keys into files.
</critical>
