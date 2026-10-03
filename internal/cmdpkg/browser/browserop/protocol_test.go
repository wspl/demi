package browserop_test

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"math"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/contract"
)

// These pure contract scenarios take less than one second and start no processes.
func TestEveryOperationDecodesItsSmallestInput(t *testing.T) {
	names := browserop.Operations()
	if len(names) != 47 {
		t.Fatalf("operations = %d, want 47", len(names))
	}
	for _, full := range names {
		name := strings.TrimPrefix(full, browserop.Prefix)
		t.Run(name, func(t *testing.T) {
			args := `{"tab":"t1"}`
			switch name {
			case "open":
				args = `{"url":"about:blank"}`
			case "tabs":
				args = `{}`
			case "content.fetch":
				args = `{"url":["https://example.test/"]}`
			case "goto":
				args = `{"tab":"t1","url":"about:blank"}`
			case "probe":
				args = `{"tab":"t1","xy":"1,2"}`
			case "drag":
				args = `{"tab":"t1","point":["1,2","3,4"]}`
			case "fill", "type", "select-text":
				args = `{"tab":"t1","text":"hello"}`
			case "key":
				args = `{"tab":"t1","key":"Enter"}`
			case "check":
				args = `{"tab":"t1","value":true}`
			case "upload":
				args = `{"tab":"t1","file":["a.txt"]}`
			case "eval":
				args = `{"tab":"t1","expression":"1"}`
			case "viewport.set":
				args = `{"tab":"t1","width":800,"height":600}`
			case "cdp.send":
				args = `{"tab":"t1","method":"Page.enable","params":"{}"}`
			case "assets.export":
				args = `{"tab":"t1","inventory":"i","output-dir":"out"}`
			case "webmcp.call":
				args = `{"tab":"t1","tool":"t","tools":"g","arguments":"{}"}`
			}
			op, err := browserop.ParseInput(name, []byte(args))
			if err != nil {
				t.Fatal(err)
			}
			if op.OperationName() != name {
				t.Fatalf("name = %s", op.OperationName())
			}
			wantTab := name != "open" && name != "tabs" && name != "content.fetch"
			if (op.TabID() != nil) != wantTab {
				t.Fatalf("tab = %v", op.TabID())
			}
			encoded, err := contract.EncodeJSON(op)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(jsonValue(t, []byte(args)), jsonValue(t, encoded)) {
				t.Fatal("decoded value differs from expected fixture")
			}
		})
	}
	_, err := browserop.ParseInput("unknown", []byte(`{}`))
	var unserved *browserop.UnservedOperation
	if !errors.As(err, &unserved) {
		t.Fatalf("error = %v", err)
	}
}

func TestInputsRefuseUnknownFieldsNullsAndOutOfBounds(t *testing.T) {
	invalid := []struct{ name, args string }{
		{"info", `{"tab":"t1","extra":1}`},
		{"info", `{"tab":"t1","timeout":null}`},
		{"info", `{"tab":"t1","timeout":0}`},
		{"info", `{"tab":"t1","timeout":300001}`},
		{"info", `{"tab":"t_short"}`},
		{"click", `{"tab":"t1","ref":"t1"}`},
		{"click", `{"tab":"t1","count":3}`},
		{"click", `{"tab":"t1","button":"back"}`},
		{"click", `{"tab":"t1","modifier":["Hyper"]}`},
		{"click", `{"tab":"t1","role":""}`},
		{"click", `{"tab":"t1","nth":1000}`},
		{"tabs", `{"limit":0}`},
		{"tabs", `{"limit":1001}`},
		{"goto", `{"tab":"t1","url":"about:blank","load":"idle"}`},
		{"drag", `{"tab":"t1","point":["1,2"]}`},
		{"content.fetch", `{"url":[]}`},
		{"content.fetch", `{"url":[` + strings.TrimSuffix(strings.Repeat(`"https://example.test/",`, 11), ",") + `]}`},
		{"viewport.set", `{"tab":"t1","width":800,"height":600,"scale":5}`},
		{"upload", `{"tab":"t1","file":[""]}`},
		{"read", `{"tab":"t1","property":"outer-html"}`},
		{"type", `{"tab":"t1","text":"` + strings.Repeat("x", 1024*1024+1) + `"}`},
	}
	for _, tt := range invalid {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := browserop.ParseInput(tt.name, []byte(tt.args)); err == nil {
				t.Fatal("invalid input accepted")
			}
		})
	}
	for _, n := range []int{4096, 4097} {
		_, err := browserop.ParseInput("click", []byte(`{"tab":"t1","css":"`+strings.Repeat("😀", n)+`"}`))
		if (err != nil) != (n > 4096) {
			t.Fatalf("%d Unicode scalars: %v", n, err)
		}
	}
}

func TestInputsAnswerTabTargetWaitAndDeadline(t *testing.T) {
	click, err := browserop.ParseInput(
		"click",
		[]byte(
			`{"tab":"t1","role":"button","name-pattern":"^Save","frame":["e2"],"nth":2,`+
				`"button":"right","wait-url":"**/done","timeout":5000}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	if *click.TabID() != "t1" || *click.WaitURLPattern() != "**/done" || click.Timeout() != 5*time.Second {
		t.Fatalf("wrong scheduling values: %#v", click)
	}
	target := click.ElementTarget()
	if target == nil || *target.Role != "button" || *target.NamePattern != "^Save" || *target.Nth != 2 ||
		!reflect.DeepEqual(*target.Frame, []browserop.NodeRef{"e2"}) {
		t.Fatalf("target = %#v", target)
	}
	input, ok := click.(*browserop.ClickInput)
	if !ok || *input.Button != browserop.MouseButtonRight {
		t.Fatalf("input = %#v", click)
	}
	open, err := browserop.ParseInput("open", []byte(`{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	if open.(*browserop.OpenInput).Load != nil || browserop.DefaultLoad != browserop.LoadDomContentLoaded {
		t.Fatal("open without load does not use DOMContentLoaded")
	}
	if open.Timeout() != 300*time.Second {
		t.Fatalf("open deadline = %v", open.Timeout())
	}
	info, err := browserop.ParseInput("info", []byte(`{"tab":"t1"}`))
	if err != nil {
		t.Fatal(err)
	}
	if info.Timeout() != 30*time.Second || info.ElementTarget() != nil {
		t.Fatalf("info = %#v", info)
	}
}

func TestQueryTreeHasOneBaseInEveryBranch(t *testing.T) {
	q, err := browserop.ParseQuery(
		[]byte(
			`{"and":[{"match":{"role":"row"}},{"match":{"text-match":"Order A"}}],` +
				`"has":{"match":{"role":"button","name":"Delete"}},"nth":0}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	target := (*q.And)[1].Match.Target()
	if target.TextMatch == nil || *target.TextMatch != "Order A" {
		t.Fatalf("target = %#v", target)
	}
	for _, bad := range []string{
		`{"match":{"role":"row"},"or":[{"match":{"role":"cell"}}]}`,
		`{"has":{"match":{"role":"row"}}}`,
		`{"match":{"role":"row"},"has":{"hasText":"x"}}`,
		`{"and":[]}`,
		`{"match":{"role":"row","nth":1}}`,
		`{"match":{"role":"row"},"visible":null}`,
	} {
		if _, err := browserop.ParseQuery([]byte(bad)); err == nil {
			t.Errorf("accepted %s", bad)
		}
	}
}

func TestInvocationDecodesToNamedOperation(t *testing.T) {
	for _, name := range []string{"browser.tabs", "browser.live"} {
		op, err := browserop.ParseOperation(name, []byte(`{}`))
		if err != nil {
			t.Fatal(err)
		}
		switch v := op.(type) {
		case *browserop.TabsInput:
			if name != "browser.tabs" || v.OperationName() != strings.TrimPrefix(name, browserop.Prefix) {
				t.Fatalf("name = %s", v.OperationName())
			}
		case *browserop.LiveInput:
			if name != "browser.live" || v.FullName() != name {
				t.Fatalf("name = %s", v.FullName())
			}
		default:
			t.Fatalf("operation = %T", op)
		}
	}
	_, err := browserop.ParseOperation("browser.live", []byte(`{"tab":"t"}`))
	var invalid *browserop.InvalidInput
	if !errors.As(err, &invalid) {
		t.Fatalf("error = %v", err)
	}
	_, err = browserop.ParseOperation("browser.nothing", []byte(`{}`))
	var unserved *browserop.UnservedOperation
	if !errors.As(err, &unserved) || unserved.Name != "browser.nothing" {
		t.Fatalf("error = %v", err)
	}
	for _, name := range []string{"file.read", "claude-code.ensure"} {
		_, err := browserop.ParseOperation(name, []byte(`{}`))
		var unknown *browserop.UnknownOperation
		if !errors.As(err, &unknown) || unknown.Name != name {
			t.Fatalf("error = %v", err)
		}
	}
	names := browserop.OperationNames()
	if len(names) != 48 {
		t.Fatalf("names = %d", len(names))
	}
	for _, name := range names {
		_, err := browserop.ParseOperation(name, []byte(`{}`))
		if err != nil && !errors.As(err, &invalid) {
			t.Fatalf("%s: %v", name, err)
		}
	}
}

func TestResultsAndFailuresPrintDocumentedNames(t *testing.T) {
	url := "https://example.test/"
	tabs := []browserop.TabID{"t1"}
	action := browserop.ActionResult{
		Operation:  "click",
		Target:     &browserop.ResolvedElement{Ref: "e2", Role: "button", Name: "Sign in"},
		Result:     json.RawMessage(`"completed"`),
		URL:        &url,
		OpenedTabs: &tabs,
	}
	encoded, err := contract.EncodeJSON(action)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"operation":"click","target":{"ref":"e2","role":"button","name":"Sign in"},` +
		`"result":"completed","url":"https://example.test/","openedTabs":["t1"]}`
	if !reflect.DeepEqual(jsonValue(t, []byte(want)), jsonValue(t, encoded)) {
		t.Fatal("decoded value differs from expected fixture")
	}
	read, err := browserop.DecodeReadResult([]byte(`{"values":[1,"a"],"truncated":false}`))
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := read.(*browserop.ReadResultAll); !ok {
		t.Fatalf("read = %T", read)
	}
	if _, err := browserop.DecodeReadResult([]byte(`{"value":1,"extra":2}`)); err == nil {
		t.Fatal("unknown field accepted")
	}
	failure := browserop.BrowserFailure{
		Code:    browserop.BrowserErrorCodePartialFailure,
		Message: "some browser items failed",
		Details: &browserop.ErrorDetails{
			Action:           new(browserop.ActionProgressNotStarted),
			Tab:              new("t1"),
			DebuggingCallers: new([]uint64{1}),
			AssetsExportResult: &browserop.AssetsExportResult{
				Directory: "/out", Manifest: "/out/manifest.json",
				Files: []browserop.ExportedAsset{{ID: "a", Path: "/out/a.png", Bytes: 3, MIMEType: "image/png"}},
			},
		},
	}
	printed, err := contract.EncodeJSON(failure)
	if err != nil {
		t.Fatal(err)
	}
	expected := `{"code":"partial_failure","message":"some browser items failed",` +
		`"details":{"action":"not_started","tab":"t1","debuggingCallers":[1],"directory":"/out",` +
		`"manifest":"/out/manifest.json","files":[{"id":"a","path":"/out/a.png","bytes":3,` +
		`"mimeType":"image/png"}]}}`
	if !reflect.DeepEqual(jsonValue(t, []byte(expected)), jsonValue(t, printed)) {
		t.Fatalf("failure JSON = %s, want %s", printed, expected)
	}
	decoded, err := browserop.DecodeBrowserFailure(printed)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(decoded, failure) {
		t.Fatalf("failure = %#v, want %#v", decoded, failure)
	}
}

func TestFrameHeadersHaveDocumentedLayout(t *testing.T) {
	if _, err := (browserop.VideoHeader{Tab: "invalid"}).Append(nil); err == nil {
		t.Fatal("accepted invalid Go tab identity")
	}
	header := browserop.VideoHeader{
		Tab:        "t1",
		Generation: 2,
		Sequence:   9,
		Key:        true,
		Timestamp:  1.5,
		Width:      1280,
		Height:     720,
	}
	data, err := header.Append(nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != browserop.VideoHeaderBytes ||
		!bytes.Equal(data[:16], []byte("t1\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00")) ||
		!bytes.Equal(data[16:25], []byte{0, 0, 0, 2, 0, 0, 0, 9, 1}) ||
		!bytes.Equal(data[36:], []byte{5, 0, 2, 208}) {
		t.Fatalf("header = %x", data)
	}
	data = append(data, []byte("data")...)
	got, payload, err := browserop.SplitVideoFrame(data)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(header, got) {
		t.Fatal("decoded value differs from expected fixture")
	}
	if string(payload) != "data" {
		t.Fatalf("payload = %q", payload)
	}
	if _, _, err := browserop.SplitVideoFrame(data[:39]); err == nil {
		t.Fatal("short video accepted")
	}
	file, payload, err := browserop.SplitFileFrame([]byte{0, 0, 0, 7, 0, 0, 0, 1, 'a'})
	if err != nil || file.Upload != 7 || file.File != 1 || string(payload) != "a" {
		t.Fatalf("file = %v %q %v", file, payload, err)
	}
	if _, _, err := browserop.SplitFileFrame(make([]byte, 7)); err == nil {
		t.Fatal("short file accepted")
	}
	capture := []byte{0, 0, 0, 3, 0, 0, 0, 4, 1, 0, 0, 0}
	capture = binary.BigEndian.AppendUint64(capture, math.Float64bits(2.5))
	capture = append(capture, 5, 0, 2, 208)
	capture = append(capture, make([]byte, 8)...)
	capture = append(capture, 0, 0, 0, 1)
	frame, payload, err := browserop.SplitCaptureFrame(capture)
	if err != nil {
		t.Fatal(err)
	}
	want := browserop.FrameHeader{Capture: 3, Sequence: 4, Key: true, Timestamp: 2.5, Width: 1280, Height: 720}
	if !reflect.DeepEqual(want, frame) {
		t.Fatal("decoded value differs from expected fixture")
	}
	if !bytes.Equal(payload, []byte{0, 0, 0, 1}) {
		t.Fatalf("payload = %x", payload)
	}
	if _, _, err := browserop.SplitCaptureFrame(capture[:31]); err == nil {
		t.Fatal("short capture accepted")
	}
}

func TestCaptureEventsDecodeAndRefuseUnknownEvents(t *testing.T) {
	for _, tt := range []struct {
		input string
		want  browserop.CaptureEvent
	}{
		{`{"type":"ready"}`, &browserop.CaptureEventReady{}},
		{`{"type":"error","capture":2,"message":"no track"}`, &browserop.CaptureEventError{Capture: 2, Message: "no track"}},
	} {
		got, err := browserop.DecodeCaptureEvent([]byte(tt.input))
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(tt.want, got) {
			t.Fatal("decoded value differs from expected fixture")
		}
	}
	for _, bad := range []string{
		`{"type":"paused","capture":1}`,
		`{"type":"started"}`,
		`{"type":"started","capture":1,"extra":true}`,
		`not json`,
	} {
		if _, err := browserop.DecodeCaptureEvent([]byte(bad)); err == nil {
			t.Errorf("accepted %s", bad)
		}
	}
}

func TestReleaseRecordsAreChecked(t *testing.T) {
	release, err := browserop.PinnedRelease()
	if err != nil {
		t.Fatal(err)
	}
	if release.Platform("aarch64-apple-darwin") == nil {
		t.Fatal("missing darwin release")
	}
	if release.Title() != "Chrome for Testing "+release.Version {
		t.Fatalf("title = %q", release.Title())
	}
	data, err := contract.EncodeJSON(release)
	if err != nil {
		t.Fatal(err)
	}
	record := jsonValue(t, data).(map[string]any)
	record["version"] = "153.0.8010"
	assertBadRelease(t, record)
	record["version"] = "153.0.8010.36"
	platform := record["platforms"].([]any)[0].(map[string]any)
	platform["sha256"] = strings.Repeat("F", 64)
	assertBadRelease(t, record)
	platform["sha256"] = strings.Repeat("f", 64)
	platform["url"] = "not a url"
	assertBadRelease(t, record)
}

func TestTabIDsAndReferencesAreNumberedFromOne(t *testing.T) {
	for _, number := range []uint64{0, 1000000000000000} {
		if _, err := browserop.NumberedTabID(number); err == nil {
			t.Errorf("accepted invalid tab number %d", number)
		}
		if _, err := browserop.NumberedNodeRef(number); err == nil {
			t.Errorf("accepted invalid node number %d", number)
		}
	}
	tab, err := browserop.NumberedTabID(7)
	if err != nil || tab != "t7" {
		t.Fatalf("tab = %q %v", tab, err)
	}
	parsed, err := browserop.ParseTabID("t7")
	if err != nil || parsed != tab {
		t.Fatalf("parsed = %q %v", parsed, err)
	}
	ref, err := browserop.NumberedNodeRef(37)
	if err != nil || ref != "e37" {
		t.Fatalf("ref = %q %v", ref, err)
	}
	for _, bad := range []string{"t", "t0", "t07", "e7", "t7a", "t-1", "t1234567890123456", "t_AAAAAAAAAAAAAAAAAAAAAA"} {
		if _, err := browserop.ParseTabID(bad); err == nil {
			t.Errorf("accepted %q", bad)
		}
	}
	if _, err := browserop.ParseNodeRef("t7"); err == nil {
		t.Fatal("accepted tab as node")
	}
	encoded, err := contract.EncodeJSON(tab)
	if err != nil || string(encoded) != `"t7"` {
		t.Fatalf("encoded = %s %v", encoded, err)
	}
}

// jsonValue compares fixture JSON independently of object member order.
func jsonValue(t *testing.T, data []byte) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(data, &value); err != nil {
		t.Fatal(err)
	}
	return value
}

// assertBadRelease checks a mutated copy of the pinned Chrome fixture.
func assertBadRelease(t *testing.T, record map[string]any) {
	t.Helper()
	data, err := contract.EncodeJSON(record)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := browserop.DecodeBrowserRelease(data); err == nil {
		t.Fatal("invalid release accepted")
	}
}

func TestLiveMessagesDecodeAsThePageSendsThem(t *testing.T) {
	data, err := os.ReadFile("testdata/live-viewer.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures struct {
		Accepted []json.RawMessage `json:"accepted"`
		Refused  []json.RawMessage `json:"refused"`
	}
	if err := json.Unmarshal(data, &fixtures); err != nil {
		t.Fatal(err)
	}
	for _, value := range fixtures.Accepted {
		decoded, err := browserop.DecodeLiveViewerMessage(value)
		if err != nil {
			t.Fatalf("%s: %v", value, err)
		}
		printed, err := contract.EncodeJSON(browserop.LiveViewerMessageJSON{Value: decoded})
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(jsonValue(t, value), jsonValue(t, printed)) {
			t.Fatalf("live message changed: got %s, want %s", printed, value)
		}
	}
	for _, value := range fixtures.Refused {
		if _, err := browserop.DecodeLiveViewerMessage(value); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
}

func TestOptionalVectorItemLimitsRemainGoOnly(t *testing.T) {
	for _, tc := range []struct {
		operation, field, rest string
		min, max               int
	}{
		{"select", "value", "", 0, browserop.StdinBytes},
		{"select", "option-label", "", 0, browserop.StdinBytes},
		{"cdp.events", "method", "", 1, browserop.LocatorLength},
		{"assets.export", "id", `,"inventory":"i","output-dir":"out"`, 1, browserop.LocatorLength},
	} {
		t.Run(tc.operation+"/"+tc.field, func(t *testing.T) {
			for _, size := range []int{0, tc.max, tc.max + 1} {
				args := []byte(`{"tab":"t1","` + tc.field + `":["` + strings.Repeat("😀", size) + `"]` + tc.rest + `}`)
				_, err := browserop.ParseInput(tc.operation, args)
				wantError := size < tc.min || size > tc.max
				if (err != nil) != wantError {
					t.Fatalf("%d Unicode scalars: %v", size, err)
				}
			}
		})
	}
}

func TestBrowserAccessorsReturnDetachedCopies(t *testing.T) {
	input, err := browserop.ParseInput(
		"click",
		[]byte(`{"tab":"t1","role":"button","frame":["e2"],"nth":2,"within":"e3","wait-url":"**/done"}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	target := input.ElementTarget()
	*target.Role = "changed"
	(*target.Frame)[0] = "e9"
	*target.Nth = 9
	*target.Within = "e9"
	*input.TabID() = "t9"
	*input.WaitURLPattern() = "changed"
	again := input.ElementTarget()
	if *again.Role != "button" || (*again.Frame)[0] != "e2" || *again.Nth != 2 || *again.Within != "e3" ||
		*input.TabID() != "t1" ||
		*input.WaitURLPattern() != "**/done" {
		t.Fatalf("mutating accessor copies changed input: %#v", input)
	}
	query, err := browserop.ParseQuery(
		[]byte(
			`{"and":[{"match":{"role":"row"}},{"match":{"text-match":"Order A"}}],` +
				`"has":{"match":{"role":"button"}},"visible":true,"nth":0}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	branches := query.Branches()
	if len(branches) != 4 || branches[1].Match.TextMatch == nil || *branches[1].Match.TextMatch != "Order A" ||
		*branches[2].Match.Role != "row" ||
		*branches[3].Match.Role != "button" {
		t.Fatalf("wrong branch traversal: %#v", branches)
	}
	*branches[0].Visible = false
	*branches[0].Nth = 5
	*(*branches[0].And)[0].Match.Role = "changed"
	*branches[1].Match.TextMatch = "changed"
	*branches[3].Match.Role = "changed"
	target = new((*query.And)[0].Match.Target())
	*target.Role = "changed too"
	if !*query.Visible || *query.Nth != 0 || *(*query.And)[0].Match.Role != "row" ||
		*(*query.And)[1].Match.TextMatch != "Order A" ||
		*query.Has.Match.Role != "button" {
		t.Fatalf("mutating query copies changed source: %#v", query)
	}
}

func TestObservationValuesKeepTheirScalarWireForm(t *testing.T) {
	for _, value := range []string{`"hello"`, `3.5`} {
		fixture := []byte(
			`{"matches":[{"ref":"e2","role":"textbox","name":"Value","value":` + value +
				`,"depth":1,"states":[]}],"count":1,"truncated":false}`,
		)
		decoded, err := browserop.DecodeFindResult(fixture)
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := contract.EncodeJSON(decoded)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(jsonValue(t, fixture), jsonValue(t, encoded)) {
			t.Fatalf("scalar changed: %s", encoded)
		}
	}
	for _, invalid := range []string{`true`, `[]`, `{}`, `null`} {
		if _, err := browserop.DecodeNodeValue([]byte(invalid)); err == nil {
			t.Errorf("accepted node value %s", invalid)
		}
	}
}
