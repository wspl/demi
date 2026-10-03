package tools

import (
	"errors"
	"fmt"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// +demi:root
// +demi:schema
type shellExecInput struct {
	Script string `json:"script"`
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
	// +demi:integer string
	ShellID   *uint64 `json:"shellId,omitempty"`
	TimeoutMS delayMS `json:"timeoutMs"`
}

// +demi:root
// +demi:schema
type commandInput struct {
	// +demi:integer string
	CommandID uint64 `json:"commandId"`
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
}

// +demi:root
// +demi:schema
type shellWriteInput struct {
	// +demi:integer string
	CommandID uint64 `json:"commandId"`
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
	// +demi:length min=1
	Stdin stdin `json:"stdin"`
}

// +demi:root
// +demi:schema
type yieldInput struct {
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
	DurationMS  delayMS `json:"durationMs"`
}

// +demi:range min=1 max=600000 schema-only
// +demi:check validateDelayMS
type delayMS uint32

// validateDelayMS preserves the shell observation and yield refusal the model reads.
func validateDelayMS(value delayMS) error {
	if value < 1 || value > 600000 {
		return fmt.Errorf("%d is not a whole number of milliseconds from 1 to 600000", value)
	}
	return nil
}

// +demi:check validateStdin
type stdin string

// validateStdin directs the model to poll instead of writing empty shell input.
func validateStdin(value stdin) error {
	if value == "" {
		return errors.New("must not be empty; use shell_status to poll")
	}
	return nil
}
