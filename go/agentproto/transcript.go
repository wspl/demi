package agentproto

import (
	"github.com/wspl/demi/go/core"
)

// One change to a transcript. A batch of patches advances the revision by
// one; a rewrite of history is one `replace`. `agent-client`'s one patch
// applier applies them.
//
//demi:union tag=op
//demi:export
type TranscriptPatch interface{ isTranscriptPatch() }

// Insert a block at the index.
//
//demi:variant add open
type TranscriptPatchAdd struct {
	Index uint32     `json:"index"`
	Value core.Block `json:"value" check:"func=core.Validate"`
}

func (TranscriptPatchAdd) isTranscriptPatch() {}

// Replace the block at the index.
//
//demi:variant replace_block open
type TranscriptPatchReplaceBlock struct {
	Index uint32     `json:"index"`
	Value core.Block `json:"value" check:"func=core.Validate"`
}

func (TranscriptPatchReplaceBlock) isTranscriptPatch() {}

// Append text to the text or thinking block at the index. Consecutive
// appends to one block merge into one.
//
//demi:variant append_text open
type TranscriptPatchAppendText struct {
	Index uint32 `json:"index"`
	Delta string `json:"delta"`
}

func (TranscriptPatchAppendText) isTranscriptPatch() {}

// Replace every block.
//
//demi:variant replace open
type TranscriptPatchReplace struct {
	Value []core.Block `json:"value" check:"each(func=core.Validate)"`
}

func (TranscriptPatchReplace) isTranscriptPatch() {}

// A transcript's version. The epoch is new each time the session's
// transcript is built, at creation and at every restore, so a version taken
// before a backend restart never matches after it; the revision counts the
// patch batches since.
//
//demi:wire
type TranscriptVersion struct {
	Epoch    string `json:"epoch" check:"chars=1.."`
	Revision uint64 `json:"revision" check:"range=..9007199254740991"`
}
