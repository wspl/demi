package core

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
)

// BlockID identifies a block; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type BlockID struct{ Identity[BlockIDKind] }
type BlockIDKind struct{ NonemptyIdentity }

func ParseBlockID(text string) (BlockID, error) {
	id, err := ParseIdentity[BlockIDKind](text)
	return BlockID{Identity: id}, err
}

// TurnID identifies a turn; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type TurnID struct{ Identity[TurnIDKind] }
type TurnIDKind struct{ NonemptyIdentity }

func ParseTurnID(text string) (TurnID, error) {
	id, err := ParseIdentity[TurnIDKind](text)
	return TurnID{Identity: id}, err
}

// NodeID identifies a node; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type NodeID struct{ Identity[NodeIDKind] }
type NodeIDKind struct{ NonemptyIdentity }

func ParseNodeID(text string) (NodeID, error) {
	id, err := ParseIdentity[NodeIDKind](text)
	return NodeID{Identity: id}, err
}

// WakeupID identifies a wakeup; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type WakeupID struct{ Identity[WakeupIDKind] }
type WakeupIDKind struct{ NonemptyIdentity }

func ParseWakeupID(text string) (WakeupID, error) {
	id, err := ParseIdentity[WakeupIDKind](text)
	return WakeupID{Identity: id}, err
}

// ShellID identifies a shell; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type ShellID struct{ Identity[ShellIDKind] }
type ShellIDKind struct{ NonemptyIdentity }

func ParseShellID(text string) (ShellID, error) {
	id, err := ParseIdentity[ShellIDKind](text)
	return ShellID{Identity: id}, err
}

// CommandID identifies a command; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type CommandID struct{ Identity[CommandIDKind] }
type CommandIDKind struct{ NonemptyIdentity }

func ParseCommandID(text string) (CommandID, error) {
	id, err := ParseIdentity[CommandIDKind](text)
	return CommandID{Identity: id}, err
}

// OperationID identifies a operation; its zero value is invalid.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type OperationID struct{ Identity[OperationIDKind] }
type OperationIDKind struct{ NonemptyIdentity }

func ParseOperationID(text string) (OperationID, error) {
	id, err := ParseIdentity[OperationIDKind](text)
	return OperationID{Identity: id}, err
}

// BlobRef names bytes by their lowercase SHA-256 digest.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","pattern":"^[0-9a-f]{64}$"}
type BlobRef struct{ Identity[BlobRefKind] }
type BlobRefKind struct{}

func (BlobRefKind) CheckIdentity(text string) error {
	refusal := errors.New("not a blob reference (64 lowercase hexadecimal digits)")
	if len(text) != 64 {
		return refusal
	}
	for _, c := range text {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return refusal
		}
	}
	return nil
}
func ParseBlobRef(text string) (BlobRef, error) {
	id, err := ParseIdentity[BlobRefKind](text)
	return BlobRef{Identity: id}, err
}
func BlobRefOf(data []byte) BlobRef {
	digest := sha256.Sum256(data)
	return BlobRef{Identity: Identity[BlobRefKind]{text: hex.EncodeToString(digest[:])}}
}

// A sequence of the numbers the model knows a conversation's things by
// (`runtime.md` § Identifiers the model sees): each is given once, in order,
// across crashes, restores and Forks.
//
//demi:enum
//demi:export
type Sequence string

const (
	SequenceCommand Sequence = "command"
	SequenceShell   Sequence = "shell"
	SequenceAgent   Sequence = "agent"
	SequenceTab     Sequence = "tab"
)
