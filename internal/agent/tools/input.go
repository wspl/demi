package tools

//go:generate go run ../../../tools/contractgen

// +demi:root
// +demi:schema
type shellExecInput struct {
	Script string `json:"script"`
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
	// +demi:integer string
	ShellID *uint64 `json:"shellId,omitempty"`
	// +demi:range min=1 max=600000
	TimeoutMS uint32 `json:"timeoutMs"`
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
	Stdin string `json:"stdin"`
}

// +demi:root
// +demi:schema
type yieldInput struct {
	// Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.
	Description *string `json:"description,omitempty"`
	// +demi:range min=1 max=600000
	DurationMS uint32 `json:"durationMs"`
}
