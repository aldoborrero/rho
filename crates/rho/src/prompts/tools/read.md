Reads files from the local filesystem.

- Reads up to 2000 lines by default
- Use `offset` and `limit` for large files
- Text output is line-number-prefixed
- Supports images (PNG, JPG) and PDFs
- For directories, returns formatted listing with modification times
- Parallelize reads when exploring related files
