package builtinproto_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/internal/schematest"
)

// schemas are the JSON Schemas the package generates, by the name of the type.
var schemas = map[string]func() jsontext.Value{
	"ActionResult":         builtinproto.ActionResultSchema,
	"AssetsExportInput":    builtinproto.AssetsExportInputSchema,
	"AssetsExportResult":   builtinproto.AssetsExportResultSchema,
	"AssetsListInput":      builtinproto.AssetsListInputSchema,
	"AssetsListResult":     builtinproto.AssetsListResultSchema,
	"BackInput":            builtinproto.BackInputSchema,
	"CapabilitiesInput":    builtinproto.CapabilitiesInputSchema,
	"CapabilitiesResult":   builtinproto.CapabilitiesResultSchema,
	"CDPDetachInput":       builtinproto.CDPDetachInputSchema,
	"CDPDetachResult":      builtinproto.CDPDetachResultSchema,
	"CDPEventsInput":       builtinproto.CDPEventsInputSchema,
	"CDPEventsResult":      builtinproto.CDPEventsResultSchema,
	"CDPSendInput":         builtinproto.CDPSendInputSchema,
	"CDPSendResult":        builtinproto.CDPSendResultSchema,
	"CDPTargetsInput":      builtinproto.CDPTargetsInputSchema,
	"CDPTargetsResult":     builtinproto.CDPTargetsResultSchema,
	"CheckInput":           builtinproto.CheckInputSchema,
	"ClickInput":           builtinproto.ClickInputSchema,
	"ClipboardReadInput":   builtinproto.ClipboardReadInputSchema,
	"ClipboardReadResult":  builtinproto.ClipboardReadResultSchema,
	"ClipboardWriteInput":  builtinproto.ClipboardWriteInputSchema,
	"ClipboardWriteResult": builtinproto.ClipboardWriteResultSchema,
	"CloseInput":           builtinproto.CloseInputSchema,
	"CloseResult":          builtinproto.CloseResultSchema,
	"ContentFetchInput":    builtinproto.ContentFetchInputSchema,
	"ContentFetchResult":   builtinproto.ContentFetchResultSchema,
	"ContentReadInput":     builtinproto.ContentReadInputSchema,
	"ContentReadResult":    builtinproto.ContentReadResultSchema,
	"CreateArgs":           builtinproto.CreateArgsSchema,
	"DialogAcceptInput":    builtinproto.DialogAcceptInputSchema,
	"DialogDismissInput":   builtinproto.DialogDismissInputSchema,
	"DialogInspectInput":   builtinproto.DialogInspectInputSchema,
	"DialogInspectResult":  builtinproto.DialogInspectResultSchema,
	"DialogResult":         builtinproto.DialogResultSchema,
	"DownloadInput":        builtinproto.DownloadInputSchema,
	"DownloadResult":       builtinproto.DownloadResultSchema,
	"DragInput":            builtinproto.DragInputSchema,
	"EditArgs":             builtinproto.EditArgsSchema,
	"EvalInput":            builtinproto.EvalInputSchema,
	"EvalResult":           builtinproto.EvalResultSchema,
	"FillInput":            builtinproto.FillInputSchema,
	"FindInput":            builtinproto.FindInputSchema,
	"FindResult":           builtinproto.FindResultSchema,
	"ForwardInput":         builtinproto.ForwardInputSchema,
	"GotoInput":            builtinproto.GotoInputSchema,
	"HistoryInput":         builtinproto.HistoryInputSchema,
	"HistoryResult":        builtinproto.HistoryResultSchema,
	"InfoInput":            builtinproto.InfoInputSchema,
	"InfoResult":           builtinproto.InfoResultSchema,
	"InspectInput":         builtinproto.InspectInputSchema,
	"InspectResult":        builtinproto.InspectResultSchema,
	"KeyInput":             builtinproto.KeyInputSchema,
	"LogsInput":            builtinproto.LogsInputSchema,
	"LogsResult":           builtinproto.LogsResultSchema,
	"MoveInput":            builtinproto.MoveInputSchema,
	"NavigationResult":     builtinproto.NavigationResultSchema,
	"OpenInput":            builtinproto.OpenInputSchema,
	"OpenResult":           builtinproto.OpenResultSchema,
	"PatchArgs":            builtinproto.PatchArgsSchema,
	"ProbeInput":           builtinproto.ProbeInputSchema,
	"ProbeResult":          builtinproto.ProbeResultSchema,
	"ReadArgs":             builtinproto.ReadArgsSchema,
	"ReadInput":            builtinproto.ReadInputSchema,
	"ReadResult":           builtinproto.ReadResultSchema,
	"ReloadInput":          builtinproto.ReloadInputSchema,
	"ScreenshotInput":      builtinproto.ScreenshotInputSchema,
	"ScreenshotResult":     builtinproto.ScreenshotResultSchema,
	"ScrollInput":          builtinproto.ScrollInputSchema,
	"SelectInput":          builtinproto.SelectInputSchema,
	"SelectTextInput":      builtinproto.SelectTextInputSchema,
	"TabsInput":            builtinproto.TabsInputSchema,
	"TabsResult":           builtinproto.TabsResultSchema,
	"TypeInput":            builtinproto.TypeInputSchema,
	"UploadInput":          builtinproto.UploadInputSchema,
	"UploadResult":         builtinproto.UploadResultSchema,
	"ViewportResetInput":   builtinproto.ViewportResetInputSchema,
	"ViewportResult":       builtinproto.ViewportResultSchema,
	"ViewportSetInput":     builtinproto.ViewportSetInputSchema,
	"WaitInput":            builtinproto.WaitInputSchema,
	"WaitResult":           builtinproto.WaitResultSchema,
	"WebmcpCallInput":      builtinproto.WebmcpCallInputSchema,
	"WebmcpCallResult":     builtinproto.WebmcpCallResultSchema,
	"WebmcpListInput":      builtinproto.WebmcpListInputSchema,
	"WebmcpListResult":     builtinproto.WebmcpListResultSchema,
}

// rustDifferences are the places where the schema of a type differs from the
// one the Rust derives (testdata/schemas), by the name of the type: the
// normalized text of a member in the Rust's schema, and the text Go generates
// for it. There is one difference. The Rust checks the elements of an optional
// list of strings (garde's inner(inner(length(...)))) and schemars leaves that
// check out of the schema; Go states every rule it checks, so the schema is
// what the input is held to.
var rustDifferences = map[string][][2]string{
	"AssetsExportInput": {{
		`"id":{"items":{"type":"string"},"type":"array"}`,
		`"id":{"items":{"maxLength":4096,"minLength":1,"type":"string"},"type":"array"}`,
	}},
	"CDPEventsInput": {{
		`"method":{"items":{"type":"string"},"type":"array"}`,
		`"method":{"items":{"maxLength":4096,"minLength":1,"type":"string"},"type":"array"}`,
	}},
	"SelectInput": {{
		`"value":{"items":{"type":"string"},"type":"array"}`,
		`"value":{"items":{"maxLength":1.048576e+06,"type":"string"},"type":"array"}`,
	}, {
		`"option-label":{"items":{"type":"string"},"type":"array"}`,
		`"option-label":{"items":{"maxLength":1.048576e+06,"type":"string"},"type":"array"}`,
	}},
}

// The schemas in testdata/schemas are what the Rust derives for the same types
// with command_schema_settings (crates/command-tree/src/input.rs): the settings
// the command tree declares every input and every --json result with. They are
// pinned data: the Rust that derived them leaves with the migration.
func TestTheSchemasAreThoseTheRustDerivesForTheSameTypes(t *testing.T) {
	files, err := filepath.Glob(filepath.Join("testdata", "schemas", "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	if len(files) != len(schemas) {
		t.Errorf("%d schemas are pinned and %d are generated", len(files), len(schemas))
	}
	for _, file := range files {
		rustName := strings.TrimSuffix(filepath.Base(file), ".json")
		// Go spells the initialism CDP where the Rust names its types Cdp...; the
		// title of a schema is the type's name.
		name := strings.Replace(rustName, "Cdp", "CDP", 1)
		generate, ok := schemas[name]
		if !ok {
			t.Errorf("%s: no schema is generated", name)
			continue
		}
		want, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		wantText, err := schematest.Normalize(want)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		wantText = strings.Replace(wantText, `"title":"`+rustName+`"`, `"title":"`+name+`"`, 1)
		for _, difference := range rustDifferences[name] {
			if !strings.Contains(wantText, difference[0]) {
				t.Errorf("%s: the Rust's schema has no %s", name, difference[0])
			}
			wantText = strings.Replace(wantText, difference[0], difference[1], 1)
		}
		gotText, err := schematest.Normalize(generate())
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if gotText != wantText {
			t.Errorf("%s: the schema differs from the Rust's\n got: %s\nwant: %s", name, gotText, wantText)
		}
	}
}

// A command's input schema is inside the command input subset, and every
// schema, an input's and a --json result's, compiles: what the command tree
// declares them with.
func TestEverySchemaCompilesAndEveryInputIsInsideTheCommandInputSubset(t *testing.T) {
	for name, generate := range schemas {
		var schema commandtree.Schema
		if err := json.Unmarshal(generate(), &schema); err != nil {
			t.Errorf("%s: %v", name, err)
			continue
		}
		if !strings.HasSuffix(name, "Input") && !strings.HasSuffix(name, "Args") {
			continue
		}
		if err := commandtree.CheckInputSubset(&schema); err != nil {
			t.Errorf("%s: %v", name, err)
		}
	}
}
