package core

import (
	"fmt"
	"strconv"
	"strings"
)

// CompletionID names one execution round of a child.
type CompletionID struct {
	Child NodeID
	Round uint64
}

func (id CompletionID) String() string {
	return fmt.Sprintf("subagent:%s:%d", id.Child, id.Round)
}

// BlockID is the block identity of the completion receipt.
func (id CompletionID) BlockID() (BlockID, error) {
	if err := id.Child.Validate(); err != nil {
		return "", fmt.Errorf("completion id: %w", err)
	}
	if id.Round > MaxSafeInteger {
		return "", fmt.Errorf("completion id: round exceeds maximum safe integer")
	}
	return ParseBlockID(id.String())
}

// NotCompletionID identifies a malformed completion receipt identity.
type NotCompletionID string

func (e NotCompletionID) Error() string {
	return fmt.Sprintf("%q is not a completion id (subagent:<child id>:<round>)", string(e))
}

// ParseCompletionID separates the child from the final decimal round.
func ParseCompletionID(text string) (CompletionID, error) {
	rest, ok := strings.CutPrefix(text, "subagent:")
	index := strings.LastIndexByte(rest, ':')
	if !ok || index < 1 || index == len(rest)-1 {
		return CompletionID{}, NotCompletionID(text)
	}
	digits := rest[index+1:]
	for _, r := range digits {
		if r < '0' || r > '9' {
			return CompletionID{}, NotCompletionID(text)
		}
	}
	round, err := strconv.ParseUint(digits, 10, 64)
	if err != nil || round > MaxSafeInteger {
		return CompletionID{}, NotCompletionID(text)
	}
	child, err := ParseNodeID(rest[:index])
	if err != nil {
		return CompletionID{}, NotCompletionID(text)
	}
	return CompletionID{Child: child, Round: round}, nil
}
