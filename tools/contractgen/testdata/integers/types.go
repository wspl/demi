package integers

// +demi:root
// +demi:schema
// +demi:msgpack
type Input struct {
	// +demi:integer string
	CommandID uint64 `json:"commandId"`
	ShellID   *ID    `json:"shellId,omitempty"`
}

// +demi:integer string
type ID uint64

// +demi:root direction=send output=protocol
// +demi:msgpack
// +demi:integer string
// +demi:range min=-5 max=5
type Small int8
