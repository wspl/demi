package core

// StreamView describes one command output stream since the last look.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type StreamView struct {
	// +demi:range max=9007199254740991
	Offset uint64 `json:"offset"`
	Delta  string `json:"delta"`
	Tail   string `json:"tail"`
	// +demi:range max=9007199254740991
	Bytes     uint64 `json:"bytes"`
	Truncated bool   `json:"truncated"`
}

// OutputView describes merged command output since the last look.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type OutputView struct {
	// +demi:range max=9007199254740991
	Offset uint64 `json:"offset"`
	// +demi:range max=9007199254740991
	Line   uint64        `json:"line"`
	Text   string        `json:"text"`
	Tail   string        `json:"tail"`
	Chunks []OutputChunk `json:"chunks"`
	// +demi:range max=9007199254740991
	Bytes     uint64 `json:"bytes"`
	Truncated bool   `json:"truncated"`
}

// BinaryStdout describes final non-text stdout without carrying its bytes.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type BinaryStdout struct {
	Truncated bool `json:"truncated"`
	// +demi:range max=9007199254740991
	TotalBytes uint64 `json:"totalBytes"`
	// +demi:range max=9007199254740991
	LimitBytes uint64 `json:"limitBytes"`
}
