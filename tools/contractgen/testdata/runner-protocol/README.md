# Runner protocol fixtures

`kept/output.msgpack` is a byte-identical copy of the kept-stream record
corpus in `internal/runnerwire/testdata/kept/`. `TestKeptTuple` decodes it
through the tuple codecs that the generator emits for the `kept` fixture
package and checks the same bytes come back. The message corpus itself is
tested once, against the real runner protocol, in `internal/runnerwire`.
