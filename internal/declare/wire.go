package declare

import "encoding/json"

//go:generate go run github.com/wspl/demi/tools/contractgen

// A command group or a command. `B` is how a native command names what it
// runs: [`Binding`] in a manifest, [`NativeOperation`] in a declaration.
//
// +demi:root
// +demi:union untagged
type rawNode interface{ rawNode() }

// A command group: a name that only selects one of its subcommands.
//
// +demi:variant rawNode
type rawGroup struct {
	Name        string    `json:"name"`
	Summary     string    `json:"summary"`
	Subcommands []rawNode `json:"subcommands"`
}

func (*rawGroup) rawNode() {}

// A leaf as the manifest writes it: `kind` names how it runs, and a native
// leaf's `binding` sits beside it.
//
// +demi:variant rawNode
type rawLeaf struct {
	Name          string           `json:"name"`
	Summary       string           `json:"summary"`
	SuccessOutput *string          `json:"successOutput,omitempty"`
	FailureOutput *string          `json:"failureOutput,omitempty"`
	RunningHint   *string          `json:"runningHint,omitempty"`
	Input         *json.RawMessage `json:"input,omitempty"`
	Positionals   *[]string        `json:"positionals,omitempty"`
	StdinField    *string          `json:"stdinField,omitempty"`
	RestField     *string          `json:"restField,omitempty"`
	Output        *rawLeafOutput   `json:"output,omitempty"`
	// +demi:enum rpc native
	Kind    string      `json:"kind"`
	Binding *rawBinding `json:"binding,omitempty"`
}

func (*rawLeaf) rawNode() {}

// The JSON Schema of a command's `--json` output.
type rawLeafOutput struct {
	JSON *json.RawMessage `json:"json,omitempty"`
}

// +demi:union untagged
type rawBinding interface{ rawBinding() }

func (*NativeOperation) rawBinding() {}
func (*Binding) rawBinding()         {}
