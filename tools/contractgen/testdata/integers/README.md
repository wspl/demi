# Numbered tool input

A tool input can declare a handle that the model writes as an integer and the
tool reads as a string, such as a command or shell id. `types.go` combines the
two forms: `commandId`, required, and `shellId`, optional.

`schema.json` is the expected JSON Schema for that input, in draft 2020-12
without a `$schema` keyword. Both properties are integers with `format`
`uint64` and `minimum` 0; the optional one also has `default: null` and is
not in `required`. `TestIntegerSchema` compares complete bytes after sorting
object keys. It keeps the root title: the tool caller removes that title,
while the generator keeps it for other schema consumers. No property
annotation is removed or changed for the comparison.

`TestIntegerStrings` pins which strings parse as an unsigned 64-bit integer:
ASCII decimal digits, leading zeros allowed, with at most one leading `+`;
no `-`, whitespace, underscore, other base, fraction, exponent or non-ASCII
digit, and nothing past the maximum. The signed fixture covers the
marker's same-integer-type rule and validation after parsing. All emitted
values remain integers.
