package builtinproto

import (
	"encoding/json/jsontext"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"
)

// smallest is the smallest input each operation decodes: its required
// members and nothing more.
func smallest(operation string) string {
	const tab = `"tab":"t1"`
	switch operation {
	case "open":
		return `{"url":"about:blank"}`
	case "tabs":
		return `{}`
	case "content.fetch":
		return `{"url":["https://example.test/"]}`
	case "goto":
		return `{` + tab + `,"url":"about:blank"}`
	case "probe":
		return `{` + tab + `,"xy":"1,2"}`
	case "drag":
		return `{` + tab + `,"point":["1,2","3,4"]}`
	case "fill", "type", "select-text":
		return `{` + tab + `,"text":"hello"}`
	case "key":
		return `{` + tab + `,"key":"Enter"}`
	case "check":
		return `{` + tab + `,"value":true}`
	case "upload":
		return `{` + tab + `,"file":["a.txt"]}`
	case "eval":
		return `{` + tab + `,"expression":"1"}`
	case "viewport.set":
		return `{` + tab + `,"width":800,"height":600}`
	case "cdp.send":
		return `{` + tab + `,"method":"Page.enable","params":"{}"}`
	case "assets.export":
		return `{` + tab + `,"inventory":"i","output-dir":"out"}`
	case "webmcp.call":
		return `{` + tab + `,"tool":"t","tools":"g","arguments":"{}"}`
	}
	return `{` + tab + `}`
}

func TestEveryOperationDecodesItsSmallestInput(t *testing.T) {
	names := Operations()
	if len(names) != 4+47+1 {
		t.Fatalf("the package serves %d operations, want 4 file operations, 47 browser operations and the live view", len(names))
	}
	browser := 0
	for _, name := range names {
		if !strings.HasPrefix(name, BrowserPrefix) || name == LiveOperation {
			continue
		}
		browser++
		short := strings.TrimPrefix(name, BrowserPrefix)
		input, err := Parse(name, []byte(smallest(short)))
		if err != nil {
			t.Errorf("%s: %v", name, err)
			continue
		}
		if _, ok := input.(BrowserInput); !ok {
			t.Errorf("%s: %T is not a BrowserInput", name, input)
		}
		_, hasTab := input.(interface{ TabID() TabID })
		if want := short != "open" && short != "tabs" && short != "content.fetch"; hasTab != want {
			t.Errorf("%s: names a tab = %v, want %v", name, hasTab, want)
		}
	}
	if browser != 47 {
		t.Errorf("%d browser operations decoded, want 47", browser)
	}
	// Every name the package lists decodes, so its descriptor lists nothing it
	// does not serve.
	for _, name := range names {
		if _, err := Parse(name, []byte(`{}`)); errors.Is(err, ErrUnknownOperation) {
			t.Errorf("%s is listed and unknown", name)
		}
	}
}

func TestInputsRefuseUnknownFieldsNullsAndValuesOutsideTheirBounds(t *testing.T) {
	long := `"` + strings.Repeat("x", 1024*1024+1) + `"`
	emoji := `"` + strings.Repeat("😀", 4097) + `"`
	for name, test := range map[string]struct{ operation, args string }{
		"an unknown field":                        {"info", `{"tab":"t1","extra":1}`},
		"a null for an optional field":            {"info", `{"tab":"t1","timeout":null}`},
		"a deadline of zero":                      {"info", `{"tab":"t1","timeout":0}`},
		"a deadline over the limit":               {"info", `{"tab":"t1","timeout":300001}`},
		"a tab that is not one":                   {"info", `{"tab":"t_short"}`},
		"a reference that is a tab":               {"click", `{"tab":"t1","ref":"t1"}`},
		"a count of three":                        {"click", `{"tab":"t1","count":3}`},
		"a button that is not one":                {"click", `{"tab":"t1","button":"back"}`},
		"a modifier that is not one":              {"click", `{"tab":"t1","modifier":["Hyper"]}`},
		"an empty role":                           {"click", `{"tab":"t1","role":""}`},
		"an index over the limit":                 {"click", `{"tab":"t1","nth":1000}`},
		"a limit of zero":                         {"tabs", `{"limit":0}`},
		"a limit over the limit":                  {"tabs", `{"limit":1001}`},
		"a load that is not one":                  {"goto", `{"tab":"t1","url":"about:blank","load":"idle"}`},
		"a drag through one point":                {"drag", `{"tab":"t1","point":["1,2"]}`},
		"no URL to fetch":                         {"content.fetch", `{"url":[]}`},
		"too many URLs to fetch":                  {"content.fetch", `{"url":[` + strings.TrimSuffix(strings.Repeat(`"a",`, 11), ",") + `]}`},
		"a scale over the limit":                  {"viewport.set", `{"tab":"t1","width":800,"height":600,"scale":5}`},
		"an empty file name":                      {"upload", `{"tab":"t1","file":[""]}`},
		"a property that is not one":              {"read", `{"tab":"t1","property":"outer-html"}`},
		"a text over the limit":                   {"type", `{"tab":"t1","text":` + long + `}`},
		"a selector over the limit in characters": {"click", `{"tab":"t1","css":` + emoji + `}`},
	} {
		if _, err := Parse("browser."+test.operation, []byte(test.args)); err == nil {
			t.Errorf("%s: %s was accepted", name, test.args)
		}
	}
	if _, err := Parse("browser.content.fetch", []byte(`{"url":[`+strings.TrimSuffix(strings.Repeat(`"a",`, 10), ",")+`]}`)); err != nil {
		t.Errorf("ten URLs to fetch: %v", err)
	}
	// A limit counts Unicode scalar values, as the page's schemas and JSON
	// Schema do: 😀 is one, though two UTF-16 units.
	if _, err := Parse("browser.click", []byte(`{"tab":"t1","css":"`+strings.Repeat("😀", 4096)+`"}`)); err != nil {
		t.Errorf("a selector at the limit: %v", err)
	}
}

func TestInputsAnswerTheirTabTargetWaitAndDeadline(t *testing.T) {
	parsed, err := Parse("browser.click", []byte(`{"tab":"t1","role":"button","name-pattern":"^Save","frame":["e2"],"nth":2,
		"button":"right","wait-url":"**/done","timeout":5000}`))
	if err != nil {
		t.Fatal(err)
	}
	click := parsed.(ClickInput)
	if click.TabID() != "t1" || *click.WaitURL != "**/done" || click.Deadline() != 5*time.Second {
		t.Errorf("tab, wait URL and deadline = %s, %s, %s", click.TabID(), *click.WaitURL, click.Deadline())
	}
	target := click.ElementTarget()
	if *target.Role != "button" || *target.NamePattern != "^Save" || !reflect.DeepEqual(*target.Frame, []NodeRef{"e2"}) || *target.Nth != 2 {
		t.Errorf("target = %+v", target)
	}
	if *click.Button != ButtonRight {
		t.Errorf("button = %s", *click.Button)
	}
	// open may take a cold start; everything else has thirty seconds.
	parsed, err = Parse("browser.open", []byte(`{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	if deadline := parsed.(OpenInput).Deadline(); deadline != 5*time.Minute {
		t.Errorf("open's deadline = %s, want 5m0s", deadline)
	}
	parsed, err = Parse("browser.info", []byte(`{"tab":"t1"}`))
	if err != nil {
		t.Fatal(err)
	}
	if deadline := parsed.(InfoInput).Deadline(); deadline != 30*time.Second {
		t.Errorf("info's deadline = %s, want 30s", deadline)
	}
	if _, targeted := parsed.(interface{ ElementTarget() BrowserTarget }); targeted {
		t.Error("info names an element target")
	}
}

func TestAQueryTreeHasOneBaseInEveryBranch(t *testing.T) {
	query, err := decode[BrowserQuery]([]byte(`{"and": [{"match": {"role": "row"}}, {"match": {"text-match": "Order A"}}],
		"has": {"match": {"role": "button", "name": "Delete"}}, "nth": 0}`))
	if err != nil {
		t.Fatal(err)
	}
	if got := *(*query.And)[1].Match.TextMatch; got != "Order A" {
		t.Errorf("the second branch matches %q", got)
	}
	for _, invalid := range []string{
		`{"match": {"role": "row"}, "or": [{"match": {"role": "cell"}}]}`,
		`{"has": {"match": {"role": "row"}}}`,
		`{"match": {"role": "row"}, "has": {"hasText": "x"}}`,
		`{"and": []}`,
		`{"match": {"role": "row", "nth": 1}}`,
		`{"match": {"role": "row"}, "visible": null}`,
	} {
		if _, err := decode[BrowserQuery]([]byte(invalid)); err == nil {
			t.Errorf("%s was accepted", invalid)
		}
	}
	_, err = decode[BrowserQuery]([]byte(`{"match": {"role": "row"}, "has": {"hasText": "x"}}`))
	if err == nil || !strings.Contains(err.Error(), "has: a query requires exactly one base: match, and, or") {
		t.Errorf("the branch without a base is named: %v", err)
	}
}

func TestFileArgumentsRefuseEmptyOldTextAndZeroPositions(t *testing.T) {
	edit := func(args string) error {
		_, err := Parse("file.edit", []byte(args))
		return err
	}
	if err := edit(`{"path":"a","old":"x","new":"y","occurrence":2}`); err != nil {
		t.Errorf("an edit of the second occurrence: %v", err)
	}
	for _, invalid := range []string{
		`{"path":"a","old":"","new":"y"}`,
		`{"path":"a","old":"x","new":"y","occurrence":0}`,
		`{"path":"a","old":"x","new":"y","context":0}`,
		`{"path":"a","old":"x","new":"y","context":null}`,
		`{"path":"a","old":"x"}`,
	} {
		if edit(invalid) == nil {
			t.Errorf("%s was accepted", invalid)
		}
	}
	if _, err := Parse("file.read", []byte(`{"path":"a","extra":true}`)); err == nil {
		t.Error("a read with an extra member was accepted")
	}
	if _, err := Parse("file.remove", []byte(`{"path":"a"}`)); !errors.Is(err, ErrUnknownOperation) {
		t.Errorf("an operation the package does not serve: %v", err)
	}
}

func TestAnInvocationDecodesToTheInputItsNameNames(t *testing.T) {
	for name, want := range map[string]any{
		"file.read":    ReadArgs{Path: "a"},
		"browser.tabs": TabsInput{},
		"browser.live": LiveInput{},
	} {
		args := `{}`
		if name == "file.read" {
			args = `{"path":"a"}`
		}
		got, err := Parse(name, []byte(args))
		if err != nil || !reflect.DeepEqual(got, want) {
			t.Errorf("%s decodes to %#v, %v, want %#v", name, got, err, want)
		}
	}
	if _, err := Parse("file.read", []byte(`{}`)); err == nil || err.Error() != "invalid: path: required" {
		t.Errorf("a refused argument names its field: %v", err)
	}
	if _, err := Parse("browser.live", []byte(`{"tab":"t"}`)); err == nil {
		t.Error("the live view took an argument")
	}
	if _, err := Parse("claude.ensure", []byte(`{}`)); !errors.Is(err, ErrUnknownOperation) {
		t.Errorf("a name of another package: %v", err)
	}
}

func TestResultsAndFailuresPrintTheDocumentedNames(t *testing.T) {
	url := "https://example.test/"
	opened := []TabID{"t1"}
	action := ActionResult{Operation: "click", Result: jsontext.Value(`"completed"`), URL: &url, OpenedTabs: &opened}
	printed, err := encode(action)
	if want := `{"operation":"click","result":"completed","url":"https://example.test/","openedTabs":["t1"]}`; err != nil || string(printed) != want {
		t.Errorf("an action prints %s, %v, want %s", printed, err, want)
	}
	all, err := decode[ReadAll]([]byte(`{"values":[1,"a"],"truncated":false}`))
	if err != nil || len(all.Values) != 2 {
		t.Errorf("a read of every match: %+v, %v", all, err)
	}
	if _, err := decode[ReadResult]([]byte(`{"values":[1,"a"],"truncated":false}`)); err != nil {
		t.Errorf("a read result: %v", err)
	}
	if _, err := decode[ReadResult]([]byte(`{"value":1,"extra":2}`)); err == nil {
		t.Error("a read result with an extra member was accepted")
	}

	action2 := ProgressNotStarted
	tab, directory, manifest := "t1", "/out", "/out/manifest.json"
	callers := []uint64{1}
	files := []ExportedAsset{{ID: "a", Path: "/out/a.png", Bytes: 3, MimeType: "image/png"}}
	failure := BrowserFailure{
		Code:    ErrPartialFailure,
		Message: "some browser items failed",
		Details: &ErrorDetails{Action: &action2, Tab: &tab, DebuggingCallers: &callers, Directory: &directory, Manifest: &manifest, Files: &files},
	}
	printed, err = encode(failure)
	want := `{"code":"partial_failure","message":"some browser items failed","details":{"action":"not_started","tab":"t1",` +
		`"debuggingCallers":[1],"directory":"/out","manifest":"/out/manifest.json",` +
		`"files":[{"id":"a","path":"/out/a.png","bytes":3,"mimeType":"image/png"}]}}`
	if err != nil || string(printed) != want {
		t.Fatalf("a failure prints %s, %v, want %s", printed, err, want)
	}
	again, err := decode[BrowserFailure](printed)
	if err != nil || !reflect.DeepEqual(again, failure) {
		t.Errorf("a failure reads back as %+v, %v", again, err)
	}
	// An export's three members come together.
	if _, err := decode[BrowserFailure]([]byte(`{"code":"partial_failure","message":"m","details":{"directory":"/out"}}`)); err == nil {
		t.Error("details with a directory and nothing else of an export were accepted")
	}
	// A dialog that is not open is null, and present.
	printed, err = encode(DialogInspectResult{})
	if err != nil || string(printed) != `{"dialog":null}` {
		t.Errorf("no dialog prints %s, %v", printed, err)
	}
	if _, err := decode[DialogInspectResult]([]byte(`{}`)); err == nil {
		t.Error("a result without its dialog was accepted")
	}
}

func TestTabIDsAndReferencesAreALetterAndANumberFromOne(t *testing.T) {
	tab := func(id string) error {
		_, err := Parse("browser.close", []byte(`{"tab":"`+id+`"}`))
		return err
	}
	reference := func(id string) error {
		_, err := Parse("browser.click", []byte(`{"tab":"t1","ref":"`+id+`"}`))
		return err
	}
	for _, valid := range []string{"t1", "t7", "t123456789012345"} {
		if err := tab(valid); err != nil {
			t.Errorf("%s: %v", valid, err)
		}
	}
	for _, invalid := range []string{"t", "t0", "t07", "e7", "t7a", "t-1", "t1234567890123456", "t_AAAAAAAAAAAAAAAAAAAAAA"} {
		if tab(invalid) == nil {
			t.Errorf("the tab %s was accepted", invalid)
		}
	}
	if err := reference("e37"); err != nil {
		t.Errorf("a reference: %v", err)
	}
	if reference("t7") == nil {
		t.Error("a tab as a reference was accepted")
	}
}
