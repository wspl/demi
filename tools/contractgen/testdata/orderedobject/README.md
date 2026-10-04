# Ordered JSON objects

A `+demi:object` field holds a JSON object as received and writes it with its
members in the order they were read.

- `refusals.jsonl`: the field's JSON Schema on the first line, then one line
  per non-object input with the decoder's refusal message. Go keeps the field
  path and the message; the source line and column are not part of its
  contract errors.
- `encodings.jsonl`: each input object and the containing contract as written,
  covering whitespace, exponent notation, negative zero, integer precision and
  escaped characters.
- `numbers.json`: float32 values written through a scalar, a list and a
  containing contract, independently of how parsed JSON numbers are normalized.

The files are recorded once; nothing regenerates them. `verify.mjs` checks the
generated Zod; `TestOrderedObjectZod` runs it with the `acceptance`
build tag.
