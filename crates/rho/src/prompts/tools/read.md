# Read

<instruction>
Reads files from the local filesystem.

- Reads up to 2000 lines by default
- Use `offset` and `limit` for large files
- Supports images (PNG, JPG) and PDFs
- For directories, returns formatted listing with modification times
- Parallelize reads when exploring related files
</instruction>

<output>
File content with line-number prefixes. Images are returned as visual content. PDFs are converted to text. Missing files return an error — do not retry without verifying the path.
</output>
