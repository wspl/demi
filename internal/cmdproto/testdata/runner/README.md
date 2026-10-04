# Runner MessagePack corpus

These eight files are runner protocol frames as the backend and the runner
exchange them, named by direction, with fields written by name. They are the
same recordings as `internal/runnerproto/testdata`, which owns the frames.

`TestRunnerMessagePackCorpus` extracts the commandwire values embedded in the
frames and checks their generated decode and encode round trips byte for byte.
Contexts cover agent and user callers, locales and language tags; package
descriptors cover executable artifacts and sorted target maps. The corpus has
no populated package resources, so those generated codecs have no recorded
value to pin. The manifest frame has an empty package map; the error artifact
location and the minimal job exit contain no location or edits to extract.
