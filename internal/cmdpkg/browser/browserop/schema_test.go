package browserop_test

import (
	"bytes"
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// TestSchemasMatchSnapshot compares every input and result of all 47
// browser leaves with testdata/schemas.json, without schema exclusions.
func TestSchemasMatchSnapshot(t *testing.T) {
	data, err := os.ReadFile("testdata/schemas.json")
	if err != nil {
		t.Fatal(err)
	}
	var reference map[string]json.RawMessage
	if err := json.Unmarshal(data, &reference); err != nil {
		t.Fatal(err)
	}
	schemas := map[string]func() json.RawMessage{
		"ContentFetchResult":   browserop.ContentFetchResultJSONSchema,
		"ProbeResult":          browserop.ProbeResultJSONSchema,
		"FindResult":           browserop.FindResultJSONSchema,
		"InspectResult":        browserop.InspectResultJSONSchema,
		"DialogInspectResult":  browserop.DialogInspectResultJSONSchema,
		"AssetsExportInput":    browserop.AssetsExportInputJSONSchema,
		"CdpEventsInput":       browserop.CDPEventsInputJSONSchema,
		"SelectInput":          browserop.SelectInputJSONSchema,
		"ActionResult":         browserop.ActionResultJSONSchema,
		"AssetsExportResult":   browserop.AssetsExportResultJSONSchema,
		"AssetsListInput":      browserop.AssetsListInputJSONSchema,
		"AssetsListResult":     browserop.AssetsListResultJSONSchema,
		"BackInput":            browserop.BackInputJSONSchema,
		"CapabilitiesInput":    browserop.CapabilitiesInputJSONSchema,
		"CapabilitiesResult":   browserop.CapabilitiesResultJSONSchema,
		"CdpDetachInput":       browserop.CDPDetachInputJSONSchema,
		"CdpDetachResult":      browserop.CDPDetachResultJSONSchema,
		"CdpEventsResult":      browserop.CDPEventsResultJSONSchema,
		"CdpSendInput":         browserop.CDPSendInputJSONSchema,
		"CdpSendResult":        browserop.CDPSendResultJSONSchema,
		"CdpTargetsInput":      browserop.CDPTargetsInputJSONSchema,
		"CdpTargetsResult":     browserop.CDPTargetsResultJSONSchema,
		"CheckInput":           browserop.CheckInputJSONSchema,
		"ClickInput":           browserop.ClickInputJSONSchema,
		"ClipboardReadInput":   browserop.ClipboardReadInputJSONSchema,
		"ClipboardReadResult":  browserop.ClipboardReadResultJSONSchema,
		"ClipboardWriteInput":  browserop.ClipboardWriteInputJSONSchema,
		"ClipboardWriteResult": browserop.ClipboardWriteResultJSONSchema,
		"CloseInput":           browserop.CloseInputJSONSchema,
		"CloseResult":          browserop.CloseResultJSONSchema,
		"ContentFetchInput":    browserop.ContentFetchInputJSONSchema,
		"ContentReadInput":     browserop.ContentReadInputJSONSchema,
		"ContentReadResult":    browserop.ContentReadResultJSONSchema,
		"DialogAcceptInput":    browserop.DialogAcceptInputJSONSchema,
		"DialogDismissInput":   browserop.DialogDismissInputJSONSchema,
		"DialogInspectInput":   browserop.DialogInspectInputJSONSchema,
		"DialogResult":         browserop.DialogResultJSONSchema,
		"DownloadInput":        browserop.DownloadInputJSONSchema,
		"DownloadResult":       browserop.DownloadResultJSONSchema,
		"DragInput":            browserop.DragInputJSONSchema,
		"EvalInput":            browserop.EvalInputJSONSchema,
		"EvalResult":           browserop.EvalResultJSONSchema,
		"FillInput":            browserop.FillInputJSONSchema,
		"FindInput":            browserop.FindInputJSONSchema,
		"ForwardInput":         browserop.ForwardInputJSONSchema,
		"GotoInput":            browserop.GotoInputJSONSchema,
		"HistoryInput":         browserop.HistoryInputJSONSchema,
		"HistoryResult":        browserop.HistoryResultJSONSchema,
		"InfoInput":            browserop.InfoInputJSONSchema,
		"InfoResult":           browserop.InfoResultJSONSchema,
		"InspectInput":         browserop.InspectInputJSONSchema,
		"KeyInput":             browserop.KeyInputJSONSchema,
		"LogsInput":            browserop.LogsInputJSONSchema,
		"LogsResult":           browserop.LogsResultJSONSchema,
		"MoveInput":            browserop.MoveInputJSONSchema,
		"NavigationResult":     browserop.NavigationResultJSONSchema,
		"OpenInput":            browserop.OpenInputJSONSchema,
		"OpenResult":           browserop.OpenResultJSONSchema,
		"ProbeInput":           browserop.ProbeInputJSONSchema,
		"ReadInput":            browserop.ReadInputJSONSchema,
		"ReadResult":           browserop.ReadResultJSONSchema,
		"ReloadInput":          browserop.ReloadInputJSONSchema,
		"ScreenshotInput":      browserop.ScreenshotInputJSONSchema,
		"ScreenshotResult":     browserop.ScreenshotResultJSONSchema,
		"ScrollInput":          browserop.ScrollInputJSONSchema,
		"SelectTextInput":      browserop.SelectTextInputJSONSchema,
		"TabsInput":            browserop.TabsInputJSONSchema,
		"TabsResult":           browserop.TabsResultJSONSchema,
		"TypeInput":            browserop.TypeInputJSONSchema,
		"UploadInput":          browserop.UploadInputJSONSchema,
		"UploadResult":         browserop.UploadResultJSONSchema,
		"ViewportResetInput":   browserop.ViewportResetInputJSONSchema,
		"ViewportResult":       browserop.ViewportResultJSONSchema,
		"ViewportSetInput":     browserop.ViewportSetInputJSONSchema,
		"WaitInput":            browserop.WaitInputJSONSchema,
		"WaitResult":           browserop.WaitResultJSONSchema,
		"WebmcpCallInput":      browserop.WebMCPCallInputJSONSchema,
		"WebmcpCallResult":     browserop.WebMCPCallResultJSONSchema,
		"WebmcpListInput":      browserop.WebMCPListInputJSONSchema,
		"WebmcpListResult":     browserop.WebMCPListResultJSONSchema,
	}
	if len(reference) != 80 || len(schemas) != len(reference) {
		t.Fatalf(
			"schema coverage: got %d, reference %d, want 80 unique schemas across 47 leaves",
			len(schemas),
			len(reference),
		)
	}
	for name := range reference {
		if schemas[name] == nil {
			t.Fatalf("missing schema %s", name)
		}
	}
	for name, schema := range schemas {
		t.Run(name, func(t *testing.T) {
			// Only the declaration builder's timeout help differs from the type.
			want := string(reference[name])
			for _, deadline := range []string{"30000", "300000"} {
				want = strings.ReplaceAll(
					want,
					"Whole operation deadline in milliseconds; default "+deadline+", maximum 300000.",
					"Whole operation deadline in milliseconds",
				)
			}
			var compact bytes.Buffer
			if err := json.Compact(&compact, []byte(want)); err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(compact.Bytes(), schema()) {
				t.Errorf("schema differs from snapshot\nwant %s\ngot %s", reference[name], schema())
			}
		})
	}
}
