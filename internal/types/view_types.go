package types

// One output stream of a command since the last look.
// +demi:tolerant
// +demi:root
type StreamView struct {
	// Where the next look starts, in bytes: just after `delta`.
	// +demi:range max=9007199254740991
	Offset uint64 `json:"offset"`
	// The text since the last look.
	Delta string `json:"delta"`
	// The end of the stream.
	Tail string `json:"tail"`
	// The stream's length so far, in bytes.
	// +demi:range max=9007199254740991
	Bytes     uint64 `json:"bytes"`
	Truncated bool   `json:"truncated"`
}

// A command's merged stdout and stderr since the last look.
// +demi:tolerant
// +demi:root
type OutputView struct {
	// Where the next model look starts, in bytes: after the whole lines in
	// `text`, or at the start of its unfinished last line while it runs.
	// +demi:range max=9007199254740991
	Offset uint64 `json:"offset"`
	// The line of the merged output that `text` starts in, from 1.
	// +demi:range max=9007199254740991
	Line uint64 `json:"line"`
	// The merged text since the last look, repeating an unfinished line.
	Text   string        `json:"text"`
	Tail   string        `json:"tail"`
	Chunks []OutputChunk `json:"chunks"`
	// +demi:range max=9007199254740991
	Bytes     uint64 `json:"bytes"`
	Truncated bool   `json:"truncated"`
}

// A command's final stdout that was not text, described by its size: its
// bytes never travel in a frame.
// +demi:tolerant
// +demi:root
type BinaryStdout struct {
	// True when the stream exceeded `limitBytes` and was cut.
	Truncated bool `json:"truncated"`
	// The stream's whole length.
	// +demi:range max=9007199254740991
	TotalBytes uint64 `json:"totalBytes"`
	// The ceiling that applied.
	// +demi:range max=9007199254740991
	LimitBytes uint64 `json:"limitBytes"`
}
