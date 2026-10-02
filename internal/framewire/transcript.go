package framewire

import "github.com/wspl/demi/internal/core"

// One change to a transcript. A batch of patches advances the revision by
// one; a rewrite of history is one `replace`. `conversation-client`'s one patch
// applier applies them.
// +demi:union tag=op
//
//sumtype:decl
type TranscriptPatch interface {
	isTranscriptPatch()
}

// Insert a block at the index.
// +demi:variant TranscriptPatch add
// +demi:tolerant
type AddPatch struct {
	Index uint32     `json:"index"`
	Value core.Block `json:"value"`
}

// Replace the block at the index.
// +demi:variant TranscriptPatch replace_block
// +demi:tolerant
type ReplaceBlockPatch struct {
	Index uint32     `json:"index"`
	Value core.Block `json:"value"`
}

// Append text to the text or thinking block at the index. Consecutive
// appends to one block merge into one.
// +demi:variant TranscriptPatch append_text
// +demi:tolerant
type AppendTextPatch struct {
	Index uint32 `json:"index"`
	Delta string `json:"delta"`
}

// Replace every block.
// +demi:variant TranscriptPatch replace
// +demi:tolerant
type ReplacePatch struct {
	Value []core.Block `json:"value"`
}

// A transcript's version. The epoch is new each time the session's
// transcript is built, at creation and at every restore, so a version taken
// before a backend restart never matches after it; the revision counts the
// patch batches since.
type TranscriptVersion struct {
	// +demi:length chars min=1
	Epoch string `json:"epoch"`
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
}
