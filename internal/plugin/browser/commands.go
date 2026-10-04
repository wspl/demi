package browser

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

const (
	summary = "Operate the conversation’s persistent browser tabs on the Host running this shell. " +
		"Use inspect to obtain node references; never guess them."
	success = "readable page results; one validated JSON value with --json"
	failure = "a browser error on stderr with nonzero exit status; an action is never replayed automatically"
)

type command struct {
	input   browserproto.Input
	result  json.RawMessage
	schema  json.RawMessage
	summary string
}

func commands() (*plugin.CommandPlugin, error) {
	specs := navigationCommands()
	specs = append(specs, observationCommands()...)
	specs = append(specs, inputCommands()...)
	specs = append(specs, textInputCommands()...)
	specs = append(specs, waitAndTransferCommands()...)
	specs = append(specs, evaluationCommands()...)
	specs = append(specs, viewportCommands()...)
	specs = append(specs, dialogCommands()...)
	specs = append(specs, debuggingCommands()...)
	specs = append(specs, contentCommands()...)
	specs = append(specs, pageToolCommands()...)

	type entry struct {
		name   string
		leaves []host.Declared
	}
	var entries []entry
	for _, spec := range specs {
		name := spec.input.OperationName()
		leaf, err := commandLeaf(spec)
		if err != nil {
			return nil, err
		}
		group, _, nested := strings.Cut(name, ".")
		if !nested {
			entries = append(entries, entry{leaves: []host.Declared{leaf}})
			continue
		}
		found := false
		for i := range entries {
			if entries[i].name == group {
				entries[i].leaves = append(entries[i].leaves, leaf)
				found = true
				break
			}
		}
		if !found {
			entries = append(entries, entry{name: group, leaves: []host.Declared{leaf}})
		}
	}
	var children []host.Declared
	for _, entry := range entries {
		if entry.name == "" {
			children = append(children, entry.leaves[0])
		} else {
			children = append(children, host.Group(entry.name, "Browser "+entry.name+" operations.", entry.leaves...))
		}
	}
	return plugin.NewCommandPlugin(plugin.PlacementDemi, []host.Declared{host.Group("browser", summary, children...)})
}

func commandLeaf(spec command) (host.Declared, error) {
	name := spec.input.OperationName()
	input, err := commanddecl.NewSchema(spec.schema)
	if err != nil {
		return host.Declared{}, fmt.Errorf("browser %s input: %w", name, err)
	}
	output, err := commanddecl.NewSchema(spec.result)
	if err != nil {
		return host.Declared{}, fmt.Errorf("browser %s result: %w", name, err)
	}
	text := success
	if name == "screenshot" {
		text = "pure PNG bytes, or file metadata with --output; --json requires --output"
	}
	parts := strings.Split(name, ".")
	positions := positionals(name)
	leaf := commanddecl.Leaf[commanddecl.NativeOperation]{
		Name:          parts[len(parts)-1],
		Summary:       spec.summary,
		Input:         input,
		Output:        &commanddecl.LeafOutput{JSON: output},
		Positionals:   &positions,
		SuccessOutput: new(text),
		FailureOutput: new(failure),
		Kind:          &commanddecl.Native[commanddecl.NativeOperation]{Binding: operation(name)},
	}
	switch name {
	case "eval":
		leaf.StdinField = new("expression")
	case "find":
		leaf.StdinField = new("body")
	case "cdp.send":
		leaf.StdinField = new("params")
	case "webmcp.call":
		leaf.StdinField = new("arguments")
	}
	return host.Leaf(leaf, nil).Describe("timeout", fmt.Sprintf(
		"Whole operation deadline in milliseconds; default %d, maximum %d.",
		spec.input.DefaultTimeoutMS(),
		browserproto.MaxTimeoutMS,
	)), nil
}

func positionals(name string) []string {
	switch name {
	case "open":
		return []string{"url"}
	case "goto":
		return []string{"tab", "url"}
	case "cdp.send":
		return []string{"tab", "method"}
	case "webmcp.call":
		return []string{"tab", "tool"}
	case "tabs", "content.fetch":
		return []string{}
	default:
		return []string{"tab"}
	}
}

func operation(name string) commanddecl.NativeOperation {
	return commanddecl.NativeOperation{Package: browserproto.Package, Operation: browserproto.Prefix + name}
}

func navigationCommands() []command {
	return []command{
		{
			&browserproto.OpenInput{},
			browserproto.OpenResultJSONSchema(),
			browserproto.OpenInputJSONSchema(),
			"Open a URL in a new tab on this Host; starts the conversation’s browser when needed.",
		},
		{
			&browserproto.TabsInput{},
			browserproto.TabsResultJSONSchema(),
			browserproto.TabsInputJSONSchema(),
			"List the conversation’s live browser tabs without starting a browser.",
		},
		{
			&browserproto.InfoInput{},
			browserproto.InfoResultJSONSchema(),
			browserproto.InfoInputJSONSchema(),
			"Read a tab’s URL, title and viewport.",
		},
		{
			&browserproto.GotoInput{},
			browserproto.NavigationResultJSONSchema(),
			browserproto.GotoInputJSONSchema(),
			"Navigate a tab to a URL.",
		},
		{
			&browserproto.BackInput{},
			browserproto.NavigationResultJSONSchema(),
			browserproto.BackInputJSONSchema(),
			"Navigate to the previous history entry.",
		},
		{
			&browserproto.ForwardInput{},
			browserproto.NavigationResultJSONSchema(),
			browserproto.ForwardInputJSONSchema(),
			"Navigate to the next history entry.",
		},
		{
			&browserproto.ReloadInput{},
			browserproto.NavigationResultJSONSchema(),
			browserproto.ReloadInputJSONSchema(),
			"Reload a tab.",
		},
		{
			&browserproto.HistoryInput{},
			browserproto.HistoryResultJSONSchema(),
			browserproto.HistoryInputJSONSchema(),
			"Read the tab’s navigation history.",
		},
		{
			&browserproto.CloseInput{},
			browserproto.CloseResultJSONSchema(),
			browserproto.CloseInputJSONSchema(),
			"Close a tab. Closing the last tab ends this browser; the next open starts fresh.",
		},
	}
}

func observationCommands() []command {
	return []command{
		{
			&browserproto.InspectInput{},
			browserproto.InspectResultJSONSchema(),
			browserproto.InspectInputJSONSchema(),
			fmt.Sprintf(
				"Read accessibility names, roles, values, states and references; at most %d nodes.",
				browserproto.MaxNodes,
			),
		},
		{
			&browserproto.FindInput{},
			browserproto.FindResultJSONSchema(),
			browserproto.FindInputJSONSchema(),
			fmt.Sprintf(
				"Find nodes by reference, role/name, associated label, visible text, test ID or CSS; at most %d nodes.",
				browserproto.MaxNodes,
			),
		},
		{
			&browserproto.ReadInput{},
			browserproto.ReadResultJSONSchema(),
			browserproto.ReadInputJSONSchema(),
			"Read a matched element’s text, HTML, value, attribute or visible/enabled/checked state.",
		},
		{
			&browserproto.ScreenshotInput{},
			browserproto.ScreenshotResultJSONSchema(),
			browserproto.ScreenshotInputJSONSchema(),
			"Capture a tab as pure PNG stdout, or save a new PNG file with --output.",
		},
		{
			&browserproto.ProbeInput{},
			browserproto.ProbeResultJSONSchema(),
			browserproto.ProbeInputJSONSchema(),
			"Find elements at viewport coordinates and optionally save an annotated screenshot.",
		},
	}
}

func inputCommands() []command {
	return []command{
		{
			&browserproto.ClickInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.ClickInputJSONSchema(),
			"Click one actionable element or an explicit viewport coordinate.",
		},
		{
			&browserproto.MoveInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.MoveInputJSONSchema(),
			"Move the pointer to an element or viewport coordinate.",
		},
		{
			&browserproto.DragInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.DragInputJSONSchema(),
			"Drag through ordered viewport points, releasing input on every exit.",
		},
		{
			&browserproto.ScrollInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.ScrollInputJSONSchema(),
			"Scroll at an element or viewport coordinate.",
		},
	}
}

func textInputCommands() []command {
	return []command{
		{
			&browserproto.FillInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.FillInputJSONSchema(),
			"Replace an editable element’s contents with text.",
		},
		{
			&browserproto.TypeInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.TypeInputJSONSchema(),
			"Type characters into a target or the current focus, preserving selection.",
		},
		{
			&browserproto.KeyInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.KeyInputJSONSchema(),
			"Press --key at the current focus or focus a supplied target first; " +
				"use a key name or a + joined combination, such as Space, Enter or ControlOrMeta+A.",
		},
		{
			&browserproto.CheckInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.CheckInputJSONSchema(),
			"Set a checkbox or radio to the requested checked value.",
		},
		{
			&browserproto.SelectInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.SelectInputJSONSchema(),
			"Select native select options by value, label or index.",
		},
		{
			&browserproto.SelectTextInput{},
			browserproto.ActionResultJSONSchema(),
			browserproto.SelectTextInputJSONSchema(),
			"Select rendered text or position its cursor; prefix and suffix disambiguate.",
		},
	}
}

func waitAndTransferCommands() []command {
	return []command{
		{
			&browserproto.WaitInput{},
			browserproto.WaitResultJSONSchema(),
			browserproto.WaitInputJSONSchema(),
			"Wait for a URL glob, element state or current-document load state, within a bounded deadline.",
		},
		{
			&browserproto.UploadInput{},
			browserproto.UploadResultJSONSchema(),
			browserproto.UploadInputJSONSchema(),
			"Attach Host files through a file input or chooser.",
		},
		{
			&browserproto.DownloadInput{},
			browserproto.DownloadResultJSONSchema(),
			browserproto.DownloadInputJSONSchema(),
			"Trigger and save a completed download on this Host.",
		},
		{
			&browserproto.ClipboardWriteInput{},
			browserproto.ClipboardWriteResultJSONSchema(),
			browserproto.ClipboardWriteInputJSONSchema(),
			"Write finite raw stdin to the managed clipboard with the declared MIME type.",
		},
		{
			&browserproto.ClipboardReadInput{},
			browserproto.ClipboardReadResultJSONSchema(),
			browserproto.ClipboardReadInputJSONSchema(),
			"Read clipboard text or export supported MIME entries to Host files.",
		},
	}
}

func evaluationCommands() []command {
	return []command{
		{
			&browserproto.EvalInput{},
			browserproto.EvalResultJSONSchema(),
			browserproto.EvalInputJSONSchema(),
			"Evaluate a read-only JavaScript expression; Chrome rejects side effects.",
		},
		{
			&browserproto.LogsInput{},
			browserproto.LogsResultJSONSchema(),
			browserproto.LogsInputJSONSchema(),
			"Read console entries without clearing them; use the returned cursor to continue.",
		},
	}
}

func viewportCommands() []command {
	return []command{
		{
			&browserproto.ViewportSetInput{},
			browserproto.ViewportResultJSONSchema(),
			browserproto.ViewportSetInputJSONSchema(),
			"Set this tab’s viewport in CSS pixels and its pixel ratio (--scale), until the user picks another mode.",
		},
		{
			&browserproto.ViewportResetInput{},
			browserproto.ViewportResultJSONSchema(),
			browserproto.ViewportResetInputJSONSchema(),
			"Return this tab to Web mode, where the user’s live view decides its size.",
		},
	}
}

func dialogCommands() []command {
	return []command{
		{
			&browserproto.DialogInspectInput{},
			browserproto.DialogInspectResultJSONSchema(),
			browserproto.DialogInspectInputJSONSchema(),
			"Read the pending JavaScript dialog.",
		},
		{
			&browserproto.DialogAcceptInput{},
			browserproto.DialogResultJSONSchema(),
			browserproto.DialogAcceptInputJSONSchema(),
			"Accept the pending JavaScript dialog, optionally supplying prompt text.",
		},
		{
			&browserproto.DialogDismissInput{},
			browserproto.DialogResultJSONSchema(),
			browserproto.DialogDismissInputJSONSchema(),
			"Dismiss the pending JavaScript dialog.",
		},
	}
}

func debuggingCommands() []command {
	return []command{
		{
			&browserproto.CDPTargetsInput{},
			browserproto.CDPTargetsResultJSONSchema(),
			browserproto.CDPTargetsInputJSONSchema(),
			"List this tab and its debuggable child targets.",
		},
		{
			&browserproto.CDPDetachInput{},
			browserproto.CDPDetachResultJSONSchema(),
			browserproto.CDPDetachInputJSONSchema(),
			"Close your debugging connection to this tab, releasing its pauses, breakpoints and interceptions.",
		},
		{
			&browserproto.CDPSendInput{},
			browserproto.CDPSendResultJSONSchema(),
			browserproto.CDPSendInputJSONSchema(),
			"Send a scoped CDP method with a JSON parameter object from stdin.",
		},
		{
			&browserproto.CDPEventsInput{},
			browserproto.CDPEventsResultJSONSchema(),
			browserproto.CDPEventsInputJSONSchema(),
			"Read buffered CDP events or wait for events after a cursor.",
		},
	}
}

func contentCommands() []command {
	return []command{
		{
			&browserproto.ContentReadInput{},
			browserproto.ContentReadResultJSONSchema(),
			browserproto.ContentReadInputJSONSchema(),
			"Read or save the page’s text or HTML content.",
		},
		{
			&browserproto.ContentFetchInput{},
			browserproto.ContentFetchResultJSONSchema(),
			browserproto.ContentFetchInputJSONSchema(),
			"Read up to ten URLs in temporary tabs sharing this browser’s login state.",
		},
		{
			&browserproto.AssetsListInput{},
			browserproto.AssetsListResultJSONSchema(),
			browserproto.AssetsListInputJSONSchema(),
			"Inventory observed page resources and inline SVGs.",
		},
		{
			&browserproto.AssetsExportInput{},
			browserproto.AssetsExportResultJSONSchema(),
			browserproto.AssetsExportInputJSONSchema(),
			"Export selected assets from a current inventory to Host files.",
		},
	}
}

func pageToolCommands() []command {
	return []command{
		{
			&browserproto.CapabilitiesInput{},
			browserproto.CapabilitiesResultJSONSchema(),
			browserproto.CapabilitiesInputJSONSchema(),
			"Report the browser’s available observation and evaluation capabilities.",
		},
		{
			&browserproto.WebMCPListInput{},
			browserproto.WebMCPListResultJSONSchema(),
			browserproto.WebMCPListInputJSONSchema(),
			"List tools registered by this page and their input schemas.",
		},
		{
			&browserproto.WebMCPCallInput{},
			browserproto.WebMCPCallResultJSONSchema(),
			browserproto.WebMCPCallInputJSONSchema(),
			"Call a registered page tool with JSON arguments from stdin.",
		},
	}
}
