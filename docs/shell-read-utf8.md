# UTF-8 read regression

The interpreter passes pipe and redirected stdin as Latin-1-packed bytes. `read`
used to assign those bytes directly to Unicode shell variables. Passing the
variable to a real process then UTF-8-encoded the byte values again, corrupting
Chinese text, emoji, and accented text. Top-level output decoding could hide the
corruption, so stdout-only tests were insufficient.

`read` now decodes byte input before assignment and IFS splitting, counts Unicode
code points for `-n`/`-N`, and translates consumed text offsets back to byte offsets
when retaining pipe input. Explicit `-u` descriptors already contain text;
descriptor redirection converts that text to the standard byte-input contract.
Read-write descriptor positions remain offsets in their stored text.

## Coverage

- `packages/just-bash/packages/just-bash/src/interpreter/builtins/read-utf8.test.ts`:
  argument values, JSONL disk bytes, multiline input, Unicode IFS, arrays, EOF,
  character counts, descriptor offsets, and text that resembles mojibake.
- `packages/just-bash/packages/just-bash/src/comparison-tests/read-utf8.comparison.test.ts`:
  comparison with native bash, including base64 verification of output bytes.
- `packages/host-local/src/__tests__/shell-read-utf8.test.ts`:
  actual child-process stdin and argv after the Demi shell read loop. Uses a
  logical cwd to isolate encoding from platform directory-fd spawn behavior.

No data repair or application deployment is part of this patch.

## Validation

- Interpreter and encoding suites: 711 passed, 1 existing skip.
- Native bash comparisons: 3 passed.
- LocalHost byte transport regression: passed.
- just-bash TypeScript, scoped Biome, knip, and full release build/pack validation: passed.
- Broader shell suite on this macOS host: 54 failures also reproduced on the
  unmodified main implementation; the patched failure set is identical. The
  local directory-fd spawn path reports ENOTDIR on this host.

## Array input

`mapfile` and its `readarray` alias decode the same byte transport before splitting
lines and storing array variables. File input, pipes, redirected groups and
heredocs preserve UTF-8 through real child-process arguments and stdin. Unicode
delimiters, retained delimiters, skip/count options, NUL separators and EOF without
a newline are covered in `mapfile-utf8.test.ts`; the LocalHost regression covers
both commands across all four input paths. No heuristic repair of already corrupt
strings is performed: literal text such as `Ã©` remains unchanged.
