# Edit

Performs exact string replacement in files.

<conditions>
- Use this tool for precise, surgical edits to existing files
- The `old_string` must match uniquely in the file — provide enough surrounding context
- Fuzzy matching handles minor whitespace and Unicode differences automatically
- For creating new files, use the Write tool instead
</conditions>

<output>
Diff showing removed and added lines, with line numbers.
</output>

<critical>
- old_string must be unique in the file. If multiple matches exist, include more surrounding lines.
- Provide the exact text including indentation, newlines, and special characters.
- No emojis unless the user explicitly requests them.
</critical>
