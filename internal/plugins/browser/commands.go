package browser

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/declare"
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
	input   browserop.Input
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
	input, err := declare.NewSchema(spec.schema)
	if err != nil {
		return host.Declared{}, fmt.Errorf("browser %s input: %w", name, err)
	}
	output, err := declare.NewSchema(spec.result)
	if err != nil {
		return host.Declared{}, fmt.Errorf("browser %s result: %w", name, err)
	}
	text := success
	if name == "screenshot" {
		text = "pure PNG bytes, or file metadata with --output; --json requires --output"
	}
	parts := strings.Split(name, ".")
	positions := positionals(name)
	leaf := declare.Leaf[declare.NativeOperation]{
		Name:          parts[len(parts)-1],
		Summary:       spec.summary,
		Input:         input,
		Output:        &declare.LeafOutput{JSON: output},
		Positionals:   &positions,
		SuccessOutput: new(text),
		FailureOutput: new(failure),
		Kind:          &declare.Native[declare.NativeOperation]{Binding: operation(name)},
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
		browserop.MaxTimeoutMS,
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

func operation(name string) declare.NativeOperation {
	return declare.NativeOperation{Package: browserop.Package, Operation: browserop.Prefix + name}
}

func navigationCommands() []command {
	return []command{
		{
			&browserop.OpenInput{},
			browserop.OpenResultJSONSchema(),
			browserop.OpenInputJSONSchema(),
			"Open a URL in a new tab on this Host; starts the conversation’s browser when needed.",
		},
		{
			&browserop.TabsInput{},
			browserop.TabsResultJSONSchema(),
			browserop.TabsInputJSONSchema(),
			"List the conversation’s live browser tabs without starting a browser.",
		},
		{
			&browserop.InfoInput{},
			browserop.InfoResultJSONSchema(),
			browserop.InfoInputJSONSchema(),
			"Read a tab’s URL, title and viewport.",
		},
		{
			&browserop.GotoInput{},
			browserop.NavigationResultJSONSchema(),
			browserop.GotoInputJSONSchema(),
			"Navigate a tab to a URL.",
		},
		{
			&browserop.BackInput{},
			browserop.NavigationResultJSONSchema(),
			browserop.BackInputJSONSchema(),
			"Navigate to the previous history entry.",
		},
		{
			&browserop.ForwardInput{},
			browserop.NavigationResultJSONSchema(),
			browserop.ForwardInputJSONSchema(),
			"Navigate to the next history entry.",
		},
		{
			&browserop.ReloadInput{},
			browserop.NavigationResultJSONSchema(),
			browserop.ReloadInputJSONSchema(),
			"Reload a tab.",
		},
		{
			&browserop.HistoryInput{},
			browserop.HistoryResultJSONSchema(),
			browserop.HistoryInputJSONSchema(),
			"Read the tab’s navigation history.",
		},
		{
			&browserop.CloseInput{},
			browserop.CloseResultJSONSchema(),
			browserop.CloseInputJSONSchema(),
			"Close a tab. Closing the last tab ends this browser; the next open starts fresh.",
		},
	}
}

func observationCommands() []command {
	return []command{
		{
			&browserop.InspectInput{},
			browserop.InspectResultJSONSchema(),
			browserop.InspectInputJSONSchema(),
			fmt.Sprintf(
				"Read accessibility names, roles, values, states and references; at most %d nodes.",
				browserop.MaxNodes,
			),
		},
		{
			&browserop.FindInput{},
			browserop.FindResultJSONSchema(),
			browserop.FindInputJSONSchema(),
			fmt.Sprintf(
				"Find nodes by reference, role/name, associated label, visible text, test ID or CSS; at most %d nodes.",
				browserop.MaxNodes,
			),
		},
		{
			&browserop.ReadInput{},
			browserop.ReadResultJSONSchema(),
			browserop.ReadInputJSONSchema(),
			"Read a matched element’s text, HTML, value, attribute or visible/enabled/checked state.",
		},
		{
			&browserop.ScreenshotInput{},
			browserop.ScreenshotResultJSONSchema(),
			browserop.ScreenshotInputJSONSchema(),
			"Capture a tab as pure PNG stdout, or save a new PNG file with --output.",
		},
		{
			&browserop.ProbeInput{},
			browserop.ProbeResultJSONSchema(),
			browserop.ProbeInputJSONSchema(),
			"Find elements at viewport coordinates and optionally save an annotated screenshot.",
		},
	}
}

func inputCommands() []command {
	return []command{
		{
			&browserop.ClickInput{},
			browserop.ActionResultJSONSchema(),
			browserop.ClickInputJSONSchema(),
			"Click one actionable element or an explicit viewport coordinate.",
		},
		{
			&browserop.MoveInput{},
			browserop.ActionResultJSONSchema(),
			browserop.MoveInputJSONSchema(),
			"Move the pointer to an element or viewport coordinate.",
		},
		{
			&browserop.DragInput{},
			browserop.ActionResultJSONSchema(),
			browserop.DragInputJSONSchema(),
			"Drag through ordered viewport points, releasing input on every exit.",
		},
		{
			&browserop.ScrollInput{},
			browserop.ActionResultJSONSchema(),
			browserop.ScrollInputJSONSchema(),
			"Scroll at an element or viewport coordinate.",
		},
	}
}

func textInputCommands() []command {
	return []command{
		{
			&browserop.FillInput{},
			browserop.ActionResultJSONSchema(),
			browserop.FillInputJSONSchema(),
			"Replace an editable element’s contents with text.",
		},
		{
			&browserop.TypeInput{},
			browserop.ActionResultJSONSchema(),
			browserop.TypeInputJSONSchema(),
			"Type characters into a target or the current focus, preserving selection.",
		},
		{
			&browserop.KeyInput{},
			browserop.ActionResultJSONSchema(),
			browserop.KeyInputJSONSchema(),
			"Press --key at the current focus or focus a supplied target first; " +
				"use a key name or a + joined combination, such as Space, Enter or ControlOrMeta+A.",
		},
		{
			&browserop.CheckInput{},
			browserop.ActionResultJSONSchema(),
			browserop.CheckInputJSONSchema(),
			"Set a checkbox or radio to the requested checked value.",
		},
		{
			&browserop.SelectInput{},
			browserop.ActionResultJSONSchema(),
			browserop.SelectInputJSONSchema(),
			"Select native select options by value, label or index.",
		},
		{
			&browserop.SelectTextInput{},
			browserop.ActionResultJSONSchema(),
			browserop.SelectTextInputJSONSchema(),
			"Select rendered text or position its cursor; prefix and suffix disambiguate.",
		},
	}
}

func waitAndTransferCommands() []command {
	return []command{
		{
			&browserop.WaitInput{},
			browserop.WaitResultJSONSchema(),
			browserop.WaitInputJSONSchema(),
			"Wait for a URL glob, element state or current-document load state, within a bounded deadline.",
		},
		{
			&browserop.UploadInput{},
			browserop.UploadResultJSONSchema(),
			browserop.UploadInputJSONSchema(),
			"Attach Host files through a file input or chooser.",
		},
		{
			&browserop.DownloadInput{},
			browserop.DownloadResultJSONSchema(),
			browserop.DownloadInputJSONSchema(),
			"Trigger and save a completed download on this Host.",
		},
		{
			&browserop.ClipboardWriteInput{},
			browserop.ClipboardWriteResultJSONSchema(),
			browserop.ClipboardWriteInputJSONSchema(),
			"Write finite raw stdin to the managed clipboard with the declared MIME type.",
		},
		{
			&browserop.ClipboardReadInput{},
			browserop.ClipboardReadResultJSONSchema(),
			browserop.ClipboardReadInputJSONSchema(),
			"Read clipboard text or export supported MIME entries to Host files.",
		},
	}
}

func evaluationCommands() []command {
	return []command{
		{
			&browserop.EvalInput{},
			browserop.EvalResultJSONSchema(),
			browserop.EvalInputJSONSchema(),
			"Evaluate a read-only JavaScript expression; Chrome rejects side effects.",
		},
		{
			&browserop.LogsInput{},
			browserop.LogsResultJSONSchema(),
			browserop.LogsInputJSONSchema(),
			"Read console entries without clearing them; use the returned cursor to continue.",
		},
	}
}

func viewportCommands() []command {
	return []command{
		{
			&browserop.ViewportSetInput{},
			browserop.ViewportResultJSONSchema(),
			browserop.ViewportSetInputJSONSchema(),
			"Set this tab’s viewport in CSS pixels and its pixel ratio (--scale), until the user picks another mode.",
		},
		{
			&browserop.ViewportResetInput{},
			browserop.ViewportResultJSONSchema(),
			browserop.ViewportResetInputJSONSchema(),
			"Return this tab to Web mode, where the user’s live view decides its size.",
		},
	}
}

func dialogCommands() []command {
	return []command{
		{
			&browserop.DialogInspectInput{},
			browserop.DialogInspectResultJSONSchema(),
			browserop.DialogInspectInputJSONSchema(),
			"Read the pending JavaScript dialog.",
		},
		{
			&browserop.DialogAcceptInput{},
			browserop.DialogResultJSONSchema(),
			browserop.DialogAcceptInputJSONSchema(),
			"Accept the pending JavaScript dialog, optionally supplying prompt text.",
		},
		{
			&browserop.DialogDismissInput{},
			browserop.DialogResultJSONSchema(),
			browserop.DialogDismissInputJSONSchema(),
			"Dismiss the pending JavaScript dialog.",
		},
	}
}

func debuggingCommands() []command {
	return []command{
		{
			&browserop.CdpTargetsInput{},
			browserop.CdpTargetsResultJSONSchema(),
			browserop.CdpTargetsInputJSONSchema(),
			"List this tab and its debuggable child targets.",
		},
		{
			&browserop.CdpDetachInput{},
			browserop.CdpDetachResultJSONSchema(),
			browserop.CdpDetachInputJSONSchema(),
			"Close your debugging connection to this tab, releasing its pauses, breakpoints and interceptions.",
		},
		{
			&browserop.CdpSendInput{},
			browserop.CdpSendResultJSONSchema(),
			browserop.CdpSendInputJSONSchema(),
			"Send a scoped CDP method with a JSON parameter object from stdin.",
		},
		{
			&browserop.CdpEventsInput{},
			browserop.CdpEventsResultJSONSchema(),
			browserop.CdpEventsInputJSONSchema(),
			"Read buffered CDP events or wait for events after a cursor.",
		},
	}
}

func contentCommands() []command {
	return []command{
		{
			&browserop.ContentReadInput{},
			browserop.ContentReadResultJSONSchema(),
			browserop.ContentReadInputJSONSchema(),
			"Read or save the page’s text or HTML content.",
		},
		{
			&browserop.ContentFetchInput{},
			browserop.ContentFetchResultJSONSchema(),
			browserop.ContentFetchInputJSONSchema(),
			"Read up to ten URLs in temporary tabs sharing this browser’s login state.",
		},
		{
			&browserop.AssetsListInput{},
			browserop.AssetsListResultJSONSchema(),
			browserop.AssetsListInputJSONSchema(),
			"Inventory observed page resources and inline SVGs.",
		},
		{
			&browserop.AssetsExportInput{},
			browserop.AssetsExportResultJSONSchema(),
			browserop.AssetsExportInputJSONSchema(),
			"Export selected assets from a current inventory to Host files.",
		},
	}
}

func pageToolCommands() []command {
	return []command{
		{
			&browserop.CapabilitiesInput{},
			browserop.CapabilitiesResultJSONSchema(),
			browserop.CapabilitiesInputJSONSchema(),
			"Report the browser’s available observation and evaluation capabilities.",
		},
		{
			&browserop.WebmcpListInput{},
			browserop.WebmcpListResultJSONSchema(),
			browserop.WebmcpListInputJSONSchema(),
			"List tools registered by this page and their input schemas.",
		},
		{
			&browserop.WebmcpCallInput{},
			browserop.WebmcpCallResultJSONSchema(),
			browserop.WebmcpCallInputJSONSchema(),
			"Call a registered page tool with JSON arguments from stdin.",
		},
	}
}
