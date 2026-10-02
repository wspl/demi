// Package transcripttest supplies predictable transcript identities and the
// model-facing texts used in black-box tests of transcript and its consumers.
package transcripttest

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

// ResumeText is what the model receives for a resume block.
const ResumeText = transcript.ResumeText

// WakeupText is what the model receives for a fired yield wakeup.
const WakeupText = transcript.WakeupText

// SequentialIDs supplies identities <prefix>-1, <prefix>-2, and onward.
// Its owner serializes calls, as with the production identity source.
type SequentialIDs struct{}

// NewSequentialIDs constructs a predictable identity source beginning at one.
func NewSequentialIDs(prefix string) *SequentialIDs { panic("not written: a-transcript") }

// NextID returns the next identity in the sequence.
func (s *SequentialIDs) NextID() string { panic("not written: a-transcript") }

// AgentMessageEnvelope returns the model-facing instruction and JSON envelope.
func AgentMessageEnvelope(message core.AgentMessage) string { panic("not written: a-transcript") }
