package browserproto_test

import (
	"bytes"
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
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
		"ContentFetchResult":   browserproto.ContentFetchResultJSONSchema,
		"ProbeResult":          browserproto.ProbeResultJSONSchema,
		"FindResult":           browserproto.FindResultJSONSchema,
		"InspectResult":        browserproto.InspectResultJSONSchema,
		"DialogInspectResult":  browserproto.DialogInspectResultJSONSchema,
		"AssetsExportInput":    browserproto.AssetsExportInputJSONSchema,
		"CdpEventsInput":       browserproto.CDPEventsInputJSONSchema,
		"SelectInput":          browserproto.SelectInputJSONSchema,
		"ActionResult":         browserproto.ActionResultJSONSchema,
		"AssetsExportResult":   browserproto.AssetsExportResultJSONSchema,
		"AssetsListInput":      browserproto.AssetsListInputJSONSchema,
		"AssetsListResult":     browserproto.AssetsListResultJSONSchema,
		"BackInput":            browserproto.BackInputJSONSchema,
		"CapabilitiesInput":    browserproto.CapabilitiesInputJSONSchema,
		"CapabilitiesResult":   browserproto.CapabilitiesResultJSONSchema,
		"CdpDetachInput":       browserproto.CDPDetachInputJSONSchema,
		"CdpDetachResult":      browserproto.CDPDetachResultJSONSchema,
		"CdpEventsResult":      browserproto.CDPEventsResultJSONSchema,
		"CdpSendInput":         browserproto.CDPSendInputJSONSchema,
		"CdpSendResult":        browserproto.CDPSendResultJSONSchema,
		"CdpTargetsInput":      browserproto.CDPTargetsInputJSONSchema,
		"CdpTargetsResult":     browserproto.CDPTargetsResultJSONSchema,
		"CheckInput":           browserproto.CheckInputJSONSchema,
		"ClickInput":           browserproto.ClickInputJSONSchema,
		"ClipboardReadInput":   browserproto.ClipboardReadInputJSONSchema,
		"ClipboardReadResult":  browserproto.ClipboardReadResultJSONSchema,
		"ClipboardWriteInput":  browserproto.ClipboardWriteInputJSONSchema,
		"ClipboardWriteResult": browserproto.ClipboardWriteResultJSONSchema,
		"CloseInput":           browserproto.CloseInputJSONSchema,
		"CloseResult":          browserproto.CloseResultJSONSchema,
		"ContentFetchInput":    browserproto.ContentFetchInputJSONSchema,
		"ContentReadInput":     browserproto.ContentReadInputJSONSchema,
		"ContentReadResult":    browserproto.ContentReadResultJSONSchema,
		"DialogAcceptInput":    browserproto.DialogAcceptInputJSONSchema,
		"DialogDismissInput":   browserproto.DialogDismissInputJSONSchema,
		"DialogInspectInput":   browserproto.DialogInspectInputJSONSchema,
		"DialogResult":         browserproto.DialogResultJSONSchema,
		"DownloadInput":        browserproto.DownloadInputJSONSchema,
		"DownloadResult":       browserproto.DownloadResultJSONSchema,
		"DragInput":            browserproto.DragInputJSONSchema,
		"EvalInput":            browserproto.EvalInputJSONSchema,
		"EvalResult":           browserproto.EvalResultJSONSchema,
		"FillInput":            browserproto.FillInputJSONSchema,
		"FindInput":            browserproto.FindInputJSONSchema,
		"ForwardInput":         browserproto.ForwardInputJSONSchema,
		"GotoInput":            browserproto.GotoInputJSONSchema,
		"HistoryInput":         browserproto.HistoryInputJSONSchema,
		"HistoryResult":        browserproto.HistoryResultJSONSchema,
		"InfoInput":            browserproto.InfoInputJSONSchema,
		"InfoResult":           browserproto.InfoResultJSONSchema,
		"InspectInput":         browserproto.InspectInputJSONSchema,
		"KeyInput":             browserproto.KeyInputJSONSchema,
		"LogsInput":            browserproto.LogsInputJSONSchema,
		"LogsResult":           browserproto.LogsResultJSONSchema,
		"MoveInput":            browserproto.MoveInputJSONSchema,
		"NavigationResult":     browserproto.NavigationResultJSONSchema,
		"OpenInput":            browserproto.OpenInputJSONSchema,
		"OpenResult":           browserproto.OpenResultJSONSchema,
		"ProbeInput":           browserproto.ProbeInputJSONSchema,
		"ReadInput":            browserproto.ReadInputJSONSchema,
		"ReadResult":           browserproto.ReadResultJSONSchema,
		"ReloadInput":          browserproto.ReloadInputJSONSchema,
		"ScreenshotInput":      browserproto.ScreenshotInputJSONSchema,
		"ScreenshotResult":     browserproto.ScreenshotResultJSONSchema,
		"ScrollInput":          browserproto.ScrollInputJSONSchema,
		"SelectTextInput":      browserproto.SelectTextInputJSONSchema,
		"TabsInput":            browserproto.TabsInputJSONSchema,
		"TabsResult":           browserproto.TabsResultJSONSchema,
		"TypeInput":            browserproto.TypeInputJSONSchema,
		"UploadInput":          browserproto.UploadInputJSONSchema,
		"UploadResult":         browserproto.UploadResultJSONSchema,
		"ViewportResetInput":   browserproto.ViewportResetInputJSONSchema,
		"ViewportResult":       browserproto.ViewportResultJSONSchema,
		"ViewportSetInput":     browserproto.ViewportSetInputJSONSchema,
		"WaitInput":            browserproto.WaitInputJSONSchema,
		"WaitResult":           browserproto.WaitResultJSONSchema,
		"WebmcpCallInput":      browserproto.WebMCPCallInputJSONSchema,
		"WebmcpCallResult":     browserproto.WebMCPCallResultJSONSchema,
		"WebmcpListInput":      browserproto.WebMCPListInputJSONSchema,
		"WebmcpListResult":     browserproto.WebMCPListResultJSONSchema,
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
