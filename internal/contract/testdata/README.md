# JSON wire bytes

These fixtures pin the exact JSON spelling Demi's wire contracts use. They are
recorded once; nothing in the repository regenerates them, and the tests only
read them.

`escaping.jsonl` covers string escaping: HTML characters, the JavaScript line
separators U+2028 and U+2029, quotes and backslashes, every ASCII control,
non-ASCII text, and literal backslash escapes. `contract.EncodeJSON` writes
these bytes; `encoding/json` would escape `<`, `>`, `&`, U+2028 and U+2029.

`numbers.jsonl` pins integer limits, overflow to floating point, negative zero,
the thresholds between plain and exponent notation, subnormal values and the
largest finite double. The tests convert these JSON values to MessagePack and
back.

Nesting depth: 126 and 127 nested arrays decode, while 128 and 129 do not. The
decoder starts with a depth budget of 128, takes one before entering an object
or array, and refuses at zero; scalars take none.
