package plugin

import (
	"fmt"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/contract"
)

// ExecutionSource is the product's reserved context source.
const ExecutionSource = "execution"

// DemiRoot is the root the repository's plugins place their groups under.
const DemiRoot = "demi"

// DemiSummary is the root's summary in command help.
const DemiSummary = "The Demi platform command: every subcommand is a platform domain (file, todo, …)."

// A plugin's id, such as `todo`: 1 to 32 lowercase letters, digits and
// hyphens. It names the plugin's context blocks, values, part of the
// product state, directories on a Host and page route.
// +demi:root
// +demi:id
// +demi:check validateID
//
//revive:disable:exported
type ID string

//revive:enable:exported

func validateID(id ID) error {
	if !validName(string(id), 32) || id == ExecutionSource {
		return fmt.Errorf(
			"\"%s\" is not a plugin id: 1 to 32 lowercase letters, digits and hyphens, not \"execution\"",
			id,
		)
	}
	return nil
}

// Schema carries a compiled declaration schema through the plugin contract.
// +demi:codec
type Schema struct{ *commanddecl.Schema }

// MarshalJSON writes the immutable schema document.
func (s Schema) MarshalJSON() ([]byte, error) {
	if s.Schema == nil {
		return nil, fmt.Errorf("plugin schema is absent")
	}
	return s.Document(), nil
}

// UnmarshalJSON compiles the schema through its owning package.
func (s *Schema) UnmarshalJSON(data []byte) error {
	v, err := commanddecl.NewSchema(data)
	if err != nil {
		return err
	}
	s.Schema = v
	return nil
}

// Declaration carries the command package's declaration codec.
// +demi:codec
type Declaration struct {
	commanddecl.Node[commanddecl.NativeOperation]
}

// MarshalJSON delegates to the declaration's existing encoder.
func (d Declaration) MarshalJSON() ([]byte, error) {
	if d.Node == nil {
		return nil, fmt.Errorf("plugin command declaration is absent")
	}
	return contract.EncodeJSON(d.Node)
}

// UnmarshalJSON delegates to the generated declaration decoder.
func (d *Declaration) UnmarshalJSON(data []byte) error {
	n, err := commanddecl.DecodeDeclaration(data)
	if err != nil {
		return err
	}
	d.Node = n
	return nil
}

// State returns the page's declaration for scope.
func (p Page) State(scope Scope) *State {
	switch scope {
	case ScopeUser:
		return p.User
	case ScopeConversation:
		return p.Conversation
	}
	return nil
}

// Scope returns the state scope marked by the topic.
func (t Topic) Scope() Scope {
	switch t {
	case TopicExposes:
		return ScopeUser
	case TopicJobs:
		return ScopeConversation
	}
	return ""
}
