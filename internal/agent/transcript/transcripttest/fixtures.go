// Package transcripttest supplies predictable transcript identities and the
// model-facing texts used in black-box tests of transcript and its consumers.
package transcripttest

import (
	"strconv"

	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

// ResumeText is what the model receives for a resume block.
const ResumeText = transcript.ResumeText

// WakeupText is what the model receives for a fired yield wakeup.
const WakeupText = transcript.WakeupText

// SequentialIDs supplies identities <prefix>-1, <prefix>-2, and onward.
// Its owner serializes calls, as with the production identity source.
type SequentialIDs struct {
	prefix string
	next   uint64
}

// NewSequentialIDs constructs a predictable identity source beginning at one.
func NewSequentialIDs(prefix string) *SequentialIDs {
	return &SequentialIDs{prefix: prefix, next: 1}
}

// NextID returns the next identity in the sequence.
func (s *SequentialIDs) NextID() string {
	id := s.next
	s.next++
	return s.prefix + "-" + strconv.FormatUint(id, 10)
}

// AgentMessageEnvelope returns the model-facing instruction and JSON envelope.
func AgentMessageEnvelope(message types.AgentMessage) string {
	return transcript.AgentMessageEnvelope(message)
}
