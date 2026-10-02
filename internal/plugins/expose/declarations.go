package expose

import (
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// commandSet binds the expose group's generated contracts to its port handlers.
func commandSet() (*plugin.CommandPlugin, error) {
	declarations := []struct {
		name, summary, success, failure string
		input, output                   json.RawMessage
		positionals                     []string
	}{
		{
			name:        "add",
			summary:     "Expose a service on a host under a fresh public URL for one hour: `demi expose add <host:port|port> [--host <name|id>]`.",
			success:     "the URL, the device, and the expiry, or JSON matching { expose } when --json is passed",
			failure:     "exposes that are not available on this instance, an unreachable --host, an address that is not host:port or a port, or a device that is not connected; writes the reason to stderr and exits non-zero",
			input:       AddArgsJSONSchema(),
			output:      ExposeAnswerJSONSchema(),
			positionals: []string{"address"},
		},
		{
			name:        "list",
			summary:     "Every expose of this user across devices, soonest expiry first.",
			success:     "one line per expose under a header, or JSON matching { exposes } when --json is passed",
			failure:     "never fails on live data",
			input:       ListArgsJSONSchema(),
			output:      ExposeLinesJSONSchema(),
			positionals: nil,
		},
		{
			name:        "renew",
			summary:     "Set an expose's expiry to one hour from now.",
			success:     "the new expiry, or JSON matching { expose } when --json is passed",
			failure:     "\"no expose <number>\" when the number names none of this user's live exposes; writes the reason to stderr and exits non-zero",
			input:       NumberArgsJSONSchema(),
			output:      ExposeAnswerJSONSchema(),
			positionals: []string{"number"},
		},
		{
			name:        "remove",
			summary:     "Destroy an expose at once; its URL no longer works.",
			success:     "confirms the removal",
			failure:     "\"no expose <number>\" when the number names none of this user's live exposes; writes the reason to stderr and exits non-zero",
			input:       NumberArgsJSONSchema(),
			output:      nil,
			positionals: []string{"number"},
		},
	}
	leaves := make([]host.Declared, 0, len(declarations))
	for _, d := range declarations {
		input, err := declare.NewSchema(d.input)
		if err != nil {
			return nil, fmt.Errorf("%s input: %w", d.name, err)
		}
		leaf := declare.Leaf[declare.NativeOperation]{Name: d.name, Summary: d.summary, SuccessOutput: &d.success, FailureOutput: &d.failure, Input: input, Kind: &declare.RPC[declare.NativeOperation]{}}
		if d.positionals != nil {
			leaf.Positionals = &d.positionals
		}
		if d.output != nil {
			output, err := declare.NewSchema(d.output)
			if err != nil {
				return nil, fmt.Errorf("%s output: %w", d.name, err)
			}
			leaf.Output = &declare.LeafOutput{JSON: output}
		}
		leaves = append(leaves, host.Leaf(leaf, plugin.PortHandled{}))
	}
	return plugin.NewCommandPlugin(plugin.PlacementDemi, []host.Declared{host.Group("expose", "Give a service on a host a public URL for one hour: add, list, renew, remove.", leaves...)})
}
