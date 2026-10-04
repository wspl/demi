# Command schemas

`types.go` declares command argument and result shapes like those of the
`file`, `browser` and `expose` commands, plus shapes only the generator's
tests need. The schemas of the real commands are pinned where they are
produced, by each plugin's manifest golden
(`internal/plugin/<id>/testdata/manifest.json`); the JSON files here pin
generator-only shapes.

## Numbers and validation

Integers of machine width get format `int` or `uint`, fixed-width integers the
format of their width, and floating point `float` or `double`. Unsigned
integers get minimum zero; only 8- and 16-bit integers get explicit width
bounds, and wider maxima are not added. Other bounds come from range and length
markers. `Numeric.json` pins this mapping, and `ExampleArgs.json` covers the
declared argument types of a command. The remaining small files exercise
generator-only shapes: unions, nullability, maps, intersected constraints and
recursive references. `Recursive` refers back to the root; `RecursiveResult`
has an inline recursive subschema plus `$defs`.

`TestCommandSchemas` compiles each tested schema as draft 2020-12 and checks
valid and invalid values against both the schema and the generated decoder,
including the file arguments that refuse empty old text and zero positions.
JSON Schema works on parsed values, so duplicate keys, integer overflow,
integer spelling and canonical timestamp spelling are checked by the decoder
alone.

## What a command schema root may hold

Recursion and timestamps are generated: `browser.inspect` output's
`BrowserTreeNode.children` refers to `#/$defs/BrowserTreeNode`, and
`ExposeLine.expiresAt` in the expose outputs is a date-time string. The
generator refuses, at a command schema root, a normalized email, HTTP URL or
trimmed string, a base64 byte contract, and a cross-field `+demi:check` hook,
because no command uses them and a schema could not state them. Browser
identifiers use explicit shared regex patterns; cursor and handle strings are
ordinary strings.

## Keyword order

Root keywords are in sorted order. Nested objects keep insertion order, and
array item schemas are reordered the same way recursively. Direct properties
keep insertion order. An internal tag is appended to the properties and
prepended to `required`.
