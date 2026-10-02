//go:build acceptance

package browser

import (
	"context"
	"encoding/json"
	"net"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/contract"
)

func TestNativeFillAndTextReplacement(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	for _, v := range []struct{ css, text string }{{"#date", "2026-09-28"}, {"#datetime", "2026-09-28T14:35"}, {"#time", "14:35"}, {"#month", "2026-09"}, {"#week", "2026-W39"}, {"#color", "#123456"}, {"#range", "42"}, {"#number", "123.5"}} {
		f.command(t, tab, "fill", browserArgs(t, `{"css":$0,"text":$1}`, v.css, v.text))
		expectValue(t, f.read(t, tab, v.css, "value"), v.text)
	}
	for _, v := range []struct{ css, text string }{{"#date", "2026-13-28"}, {"#datetime", "2026-13-28T14:35"}, {"#number", "not a number"}, {"#time", "99:99"}} {
		f.rejects(t, tab, "fill", browserArgs(t, `{"css":$0,"text":$1}`, v.css, v.text), "invalid_input", "not_started")
	}
	expectValue(t, f.read(t, tab, "#date", "value"), "2026-09-28")
	for _, css := range []string{"#text", "#textarea", "#editable"} {
		property := "value"
		if css == "#editable" {
			property = "text-content"
		}
		for _, text := range []string{"replacement", ""} {
			f.command(t, tab, "fill", browserArgs(t, `{"css":$0,"text":$1}`, css, text))
			expectValue(t, f.read(t, tab, css, property), text)
		}
	}
	for _, css := range []string{"#plain", "#file", "#readonly"} {
		f.rejects(t, tab, "fill", browserArgs(t, `{"css":$0,"text":"x","timeout":500}`, css), "not_actionable", "not_started")
	}
	f.command(t, tab, "fill", `{"css":"#covered-input","text":"under overlay"}`)
	expectValue(t, f.read(t, tab, "#covered-input", "value"), "under overlay")
	f.click(t, tab, "#transform-text")
	f.command(t, tab, "fill", `{"css":"#text","text":"transformed"}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "TRANSFORMED")
	f.click(t, tab, "#prevent-text")
	f.command(t, tab, "fill", `{"css":"#text","text":""}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "TRANSFORMED")
}

func TestTargetedKeyboardPreservesSelectionAndStopsOnFocusLoss(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	for _, prepare := range []string{"#select-middle", "#select-blurred"} {
		f.command(t, tab, "fill", `{"css":"#text","text":"hello"}`)
		f.click(t, tab, prepare)
		f.command(t, tab, "type", `{"css":"#text","text":"XY"}`)
		expectValue(t, f.read(t, tab, "#text", "value"), "heXYo")
	}
	f.click(t, tab, "#focus-other")
	f.command(t, tab, "key", `{"css":"#unchecked","key":"Space"}`)
	expectValue(t, f.read(t, tab, "#unchecked", "checked"), true)
	expectValue(t, f.read(t, tab, "#other", "value"), "elsewhere")
	f.command(t, tab, "key", `{"css":"#text","key":"ControlOrMeta+A"}`)
	f.command(t, tab, "type", `{"css":"#text","text":"AB"}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "AB")
	for _, key := range []string{"+", "Shift+a", "Shift+Digit1"} {
		f.command(t, tab, "key", browserArgs(t, `{"css":"#text","key":$0}`, key))
	}
	expectValue(t, f.read(t, tab, "#text", "value"), "AB+A!")
	f.rejects(t, tab, "key", `{"css":"#text","key":"Control+NotAKey"}`, "invalid_input", "not_started")
	expectValue(t, f.eval(t, tab, `keys.length===28 && keys.filter(e=>e.type==='keydown').length===keys.filter(e=>e.type==='keyup').length && keys.some((e,i)=>e.key==='X'&&e.type==='keydown'&&keys[i+1]?.key==='X'&&keys[i+1]?.type==='keyup')`), true)
	for _, trigger := range []string{"#lose-focus", "#replace-target"} {
		f.click(t, tab, trigger)
		details := f.rejects(t, tab, "type", `{"css":"#text","text":"XYZ"}`, "not_actionable", "completed")
		if details.Delivered == nil || *details.Delivered != 1 {
			t.Fatal(details)
		}
		expectValue(t, f.read(t, tab, "#other", "value"), "elsewhere")
		f.command(t, tab, "reload", `{}`)
	}
}

func TestCheckVerifiesStateAndSelectWaitsForOrderedOptions(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	f.command(t, tab, "check", `{"css":"#checked","value":true}`)
	expectValue(t, f.read(t, tab, "#checked", "checked"), true)
	for _, test := range []struct {
		css, code, action string
		value             bool
	}{{"#prevented", "not_actionable", "completed", true}, {"#radio", "invalid_input", "not_started", false}, {"#plain", "not_actionable", "not_started", true}} {
		f.rejects(t, tab, "check", browserArgs(t, `{"css":$0,"value":$1}`, test.css, test.value), test.code, test.action)
	}
	for _, css := range []string{"#aria-check", "#aria-switch"} {
		f.command(t, tab, "check", browserArgs(t, `{"css":$0,"value":true}`, css))
		expectValue(t, f.read(t, tab, css, "checked"), true)
	}
	f.click(t, tab, "#add-late")
	for _, test := range []struct{ args, want string }{
		{`{"css":"#late","value":["new"]}`, `[{"value":"new","label":"Late"}]`},
		{`{"css":"#single","value":["two","one"]}`, `[{"value":"one","label":"First"}]`},
		{`{"css":"#single","value":["missing","two"]}`, `[{"value":"two","label":"Second"}]`},
		{`{"css":"#multi","value":["two","one"]}`, `[{"value":"one","label":"First"},{"value":"two","label":"Second"}]`},
		{`{"css":"#single","option-label":["Second"]}`, `[{"value":"two","label":"Second"}]`},
		{`{"css":"#hidden-select","option-index":[1]}`, `[{"value":"two","label":"Second"}]`},
	} {
		got := observedField(t, f.command(t, tab, "select", test.args), "result")
		if string(got) != test.want {
			t.Fatalf("%s: %s", test.args, got)
		}
	}
	for _, value := range []string{"bad", "group"} {
		f.rejects(t, tab, "select", browserArgs(t, `{"css":"#disabled-option","value":[$0],"timeout":500}`, value), "not_actionable", "not_started")
	}
	f.rejects(t, tab, "select", `{"css":"#single","value":["missing"],"timeout":500}`, "target_not_found", "not_started")
	f.rejects(t, tab, "select", `{"css":"#single","value":["one"],"option-index":[0]}`, "invalid_input", "not_started")
}

func TestLabelTextAndAccessibleNameAreDistinctAndAmbiguityIsExplicit(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	for _, role := range []string{"Date", "date", "DATE"} {
		f.command(t, tab, "fill", browserArgs(t, `{"role":$0,"name":"Appointment date","exact":true,"text":"2026-09-28"}`, role))
		expectValue(t, f.read(t, tab, "#date", "value"), "2026-09-28")
	}
	for _, test := range []struct{ label, css string }{{"Appointment date", "#date"}, {"Wrapped date", "#wrapped"}, {"Related date", "#related"}} {
		f.command(t, tab, "fill", browserArgs(t, `{"label":$0,"exact":true,"text":"2026-10-01"}`, test.label))
		expectValue(t, f.read(t, tab, test.css, "value"), "2026-10-01")
	}
	for _, test := range []struct {
		args  string
		count int
	}{
		{`{"label":"File label","exact":true}`, 1}, {`{"label":"Text field","exact":true}`, 0},
		{`{"text-match":"Accessible label","exact":true}`, 0}, {`{"text-match":"Visible words","exact":true}`, 1},
		{`{"role":"button","name":"Accessible label","exact":true}`, 1}, {`{"role":"button","name":"Visible words","exact":true}`, 0},
		{`{"role":"button","name":"AX fallback","exact":true}`, 2}, {`{"role":"button","name":"Fallback","exact":true}`, 1},
		{`{"role":"button","name":"Fallback","exact":true,"nth":1}`, 0}, {`{"role":"button","name":"Hidden duplicate","exact":true}`, 0},
	} {
		expectValue(t, observedField(t, f.command(t, tab, "find", test.args), "count"), test.count)
	}
	for _, css := range []string{".ax-fallback", ".fallback"} {
		expectValue(t, observedField(t, f.command(t, tab, "read", browserArgs(t, `{"css":$0,"property":"visible","all":true}`, css)), "values"), []bool{false, true})
	}
	f.command(t, tab, "click", `{"role":"button","name":"AX fallback","exact":true}`)
	expectValue(t, f.eval(t, tab, "fallbackClicks"), 1)
	f.click(t, tab, ".fallback")
	expectValue(t, f.eval(t, tab, "fallbackClicks"), 2)
	for _, css := range []string{".ambiguous", ".all-hidden"} {
		f.rejects(t, tab, "click", browserArgs(t, `{"css":$0}`, css), "ambiguous_target", "not_started")
	}
	f.rejects(t, tab, "click", `{"css":".fallback","nth":0,"timeout":500}`, "not_actionable", "not_started")
	f.rejects(t, tab, "read", `{"css":"#missing","property":"text"}`, "target_not_found", "not_started")
	f.rejects(t, tab, "click", `{"css":"#missing","timeout":500}`, "target_not_found", "not_started")
	scope := observedField(t, f.command(t, tab, "find", `{"css":"#scope"}`), "matches", "0", "ref")
	expectValue(t, observedField(t, f.command(t, tab, "find", browserArgs(t, `{"role":"button","name":"Scoped","within":$0}`, scope)), "count"), 1)
}

func TestInspectKeepsFalseValuesAndProtectsPasswordsAndHandlesExpire(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	tree := string(f.command(t, tab, "inspect", `{"limit":1000}`))
	for _, part := range []string{`"hello"`, `"protected"`, `"checked=false"`} {
		if !strings.Contains(tree, part) {
			t.Fatalf("missing %s in %s", part, tree)
		}
	}
	if strings.Contains(tree, "never-print-this-password") {
		t.Fatal("password leaked")
	}
	f.rejects(t, tab, "read", `{"css":"#password","property":"value"}`, "protected_value", "not_started")
	reference := observedField(t, f.command(t, tab, "find", `{"css":"#text"}`), "matches", "0", "ref")
	again := observedField(t, f.command(t, tab, "find", `{"css":"#text"}`), "matches", "0", "ref")
	if string(reference) != string(again) {
		t.Fatal("reference changed in same document")
	}
	f.click(t, tab, "#no-nav")
	f.command(t, tab, "fill", browserArgs(t, `{"ref":$0,"text":"survives DOM update"}`, reference))
	f.command(t, tab, "reload", `{}`)
	renewed := observedField(t, f.command(t, tab, "find", `{"css":"#text"}`), "matches", "0", "ref")
	if string(reference) == string(renewed) {
		t.Fatal("reference reused across documents")
	}
	f.rejects(t, tab, "fill", browserArgs(t, `{"ref":$0,"text":"stale"}`, reference), "stale_ref", "not_started")
	f.rejects(t, tab, "wait", browserArgs(t, `{"ref":$0,"state":"hidden"}`, reference), "stale_ref", "not_started")
	f.command(t, tab, "close", `{}`)
	next := f.open(t, "repairs.html")
	if next == tab {
		t.Fatal("tab reused")
	}
	f.rejects(t, next, "fill", browserArgs(t, `{"ref":$0,"text":"stale"}`, reference), "stale_ref", "not_started")
	f.failure(t, "info", browserArgs(t, `{"tab":$0}`, tab), "tab_not_found")
}

// This scenario retains Rust's five-second actionability budgets; total budget 45 s.
func TestActionConditionsShadowHitsAndSharedWaitStates(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "repairs.html")
	details := f.rejects(t, tab, "click", `{"css":"#covered-button","timeout":5000}`, "not_actionable", "not_started")
	if details.Interceptor == nil || !strings.Contains(*details.Interceptor, "button-overlay") {
		t.Fatal(details)
	}
	for _, test := range []struct{ css, condition string }{{"#fieldset-button", "enabled"}, {"#pointer-none", "pointer-events"}, {"#moving", "stable"}} {
		d := f.rejects(t, tab, "click", browserArgs(t, `{"css":$0,"timeout":5000}`, test.css), "not_actionable", "not_started")
		if d.Condition == nil || *d.Condition != test.condition {
			t.Fatal(d)
		}
	}
	f.command(t, tab, "move", `{"css":"#disabled-button"}`)
	expectValue(t, f.eval(t, tab, "hovered"), true)
	f.command(t, tab, "scroll", `{"css":"#disabled-button","dy":50}`)
	for _, test := range []struct{ name, args string }{{"click", `{"xy":"99999,99999"}`}, {"click", `{"css":"#no-nav","xy":"10,10"}`}, {"wait", `{"url":"**","css":"#no-nav","state":"visible"}`}, {"scroll", `{"dy":0}`}} {
		f.rejects(t, tab, test.name, test.args, "invalid_input", "not_started")
	}
	for _, test := range []struct{ before, after string }{{"Shadow button", "Shadow clicked"}, {"Frame button", "Clicked frame"}} {
		f.command(t, tab, "click", browserArgs(t, `{"role":"button","name":$0,"exact":true}`, test.before))
		expectValue(t, observedField(t, f.command(t, tab, "find", browserArgs(t, `{"role":"button","name":$0,"exact":true}`, test.after)), "count"), 1)
	}
	date := observedField(t, f.command(t, tab, "find", `{"css":"#date"}`), "matches", "0", "ref")
	nodes, err := browserop.DecodeFindResult(f.command(t, tab, "find", browserArgs(t, `{"role":"spinbutton","within":$0}`, date)))
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, node := range nodes.Matches {
		raw := mustBrowserValue(t, node)
		year, _ := contract.Decode[float64](observedField(t, raw, "value"))
		if year == 2026 {
			ref := observedField(t, raw, "ref")
			f.command(t, tab, "click", browserArgs(t, `{"ref":$0}`, ref))
			f.rejects(t, tab, "fill", browserArgs(t, `{"ref":$0,"text":"2027"}`, ref), "not_actionable", "not_started")
			found = true
			break
		}
	}
	if !found {
		t.Fatal("date year reference missing")
	}
	f.rejects(t, tab, "wait", `{"css":"#hidden","state":"visible","timeout":500}`, "timeout", "not_started")
	for _, state := range []string{"hidden", "attached"} {
		f.command(t, tab, "wait", browserArgs(t, `{"css":"#hidden","state":$0}`, state))
	}
	for _, css := range []string{"#fieldset-input", "#aria-disabled"} {
		f.rejects(t, tab, "wait", browserArgs(t, `{"css":$0,"state":"enabled","timeout":500}`, css), "timeout", "not_started")
		f.rejects(t, tab, "fill", browserArgs(t, `{"css":$0,"text":"x","timeout":5000}`, css), "not_actionable", "not_started")
	}
	f.click(t, tab, "#show-later")
	f.command(t, tab, "wait", `{"css":"#hidden","state":"visible"}`)
	f.click(t, tab, "#enable-later")
	f.command(t, tab, "wait", `{"css":"#aria-disabled","state":"enabled"}`)
	removable := observedField(t, f.command(t, tab, "find", `{"css":"#removable"}`), "matches", "0", "ref")
	f.click(t, tab, "#remove-later")
	for _, state := range []string{"detached", "hidden"} {
		f.command(t, tab, "wait", browserArgs(t, `{"ref":$0,"state":$1}`, removable, state))
	}
	f.rejects(t, tab, "click", browserArgs(t, `{"ref":$0}`, removable), "stale_ref", "not_started")
}

func TestTextLocatorsScanLargeDocumentsShadowRootsAndFrames(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "many")
	for _, label := range []string{"Appointment date", "Open shadow label", "Frame review label"} {
		found := f.command(t, tab, "find", browserArgs(t, `{"label":$0,"exact":true}`, label))
		expectValue(t, observedField(t, found, "count"), 1)
		ref := observedField(t, found, "matches", "0", "ref")
		f.command(t, tab, "fill", browserArgs(t, `{"ref":$0,"text":"2026-10-01"}`, ref))
	}
	for _, text := range []string{"Visible words", "Open shadow text", "Frame button"} {
		found := f.command(t, tab, "find", browserArgs(t, `{"text-match":$0,"exact":true}`, text))
		expectValue(t, observedField(t, found, "count"), 1)
	}
	scope := observedField(t, f.command(t, tab, "find", `{"css":"#open-shadow"}`), "matches", "0", "ref")
	expectValue(t, observedField(t, f.command(t, tab, "find", browserArgs(t, `{"label":"Open shadow label","within":$0}`, scope)), "count"), 1)
}

func TestExplicitNavigationTracksDocumentsFailuresAndHistoryBoundaries(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/500"))
	expectValue(t, f.eval(t, tab, "document.title"), "HTTP error document")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/redirect"))
	expectValue(t, f.eval(t, tab, "location.pathname"), "/destination")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/destination#changed"))
	f.command(t, tab, "back", `{}`)
	expectValue(t, f.eval(t, tab, "location.hash"), "")
	f.command(t, tab, "forward", `{}`)
	expectValue(t, f.eval(t, tab, "location.hash"), "#changed")
	for _, load := range []string{"commit", "domcontentloaded", "load"} {
		f.command(t, tab, "goto", browserArgs(t, `{"url":$0,"load":$1}`, f.url, load))
		f.command(t, tab, "reload", browserArgs(t, `{"load":$0}`, load))
	}
	reserved, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	refused := "http://" + reserved.Addr().String() + "/"
	if err := reserved.Close(); err != nil {
		t.Fatal(err)
	}
	failure := f.failure(t, "goto", browserArgs(t, `{"tab":$0,"url":$1}`, tab, refused), "navigation_failed")
	if !strings.Contains(failure.Error.Message, "ERR_CONNECTION_REFUSED") {
		t.Fatal(failure)
	}
	f.rejects(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/drop"), "navigation_failed", "completed")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/reload-drop"))
	f.rejects(t, tab, "reload", `{}`, "navigation_failed", "completed")
	opened, err := browserop.DecodeOpenResult(f.call(t, "open", `{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"back", "forward"} {
		f.rejects(t, opened.Tab, name, `{}`, "history_boundary", "not_started")
	}
	entries, err := contract.List(observedField(t, f.command(t, opened.Tab, "history", `{}`), "entries"), contract.Decode[json.RawMessage])
	if err != nil || len(entries) != 1 {
		t.Fatalf("%s %v", entries, err)
	}
	timeout := f.failure(t, "open", browserArgs(t, `{"url":$0,"timeout":700}`, f.url+"/stall"), "timeout")
	if timeout.Error.Details == nil || timeout.Error.Details.Tab == nil {
		t.Fatal(timeout)
	}
}

func TestCatalogQueriesPatternsPaginationAndReadOnlyElements(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	matches := f.command(t, tab, "find", `{"role":"button","name-pattern":"^Delete$","offset":1,"limit":1}`)
	expectValue(t, observedField(t, matches, "count"), 2)
	expectValue(t, observedField(t, matches, "truncated"), false)
	rows, err := contract.List(observedField(t, matches, "matches"), contract.Decode[json.RawMessage])
	if err != nil || len(rows) != 1 {
		t.Fatalf("%s %v", rows, err)
	}
	query := `{"within":{"match":{"css":".catalog-row"},"hasText":"Order A"},"match":{"role":"button","name":"Delete","exact":true},"visible":true}`
	found := f.command(t, tab, "find", browserArgs(t, `{"query":true,"body":$0}`, query))
	expectValue(t, observedField(t, found, "count"), 1)
	ref := observedField(t, found, "matches", "0", "ref")
	result := f.command(t, tab, "eval", browserArgs(t, `{"ref":$0,"expression":"element.parentElement.innerText"}`, ref))
	if !strings.Contains(string(observedField(t, result, "value")), "Order A") {
		t.Fatal(string(result))
	}
	root := observedField(t, f.command(t, tab, "inspect", `{}`), "tree", "0", "ref")
	expectValue(t, observedField(t, f.command(t, tab, "eval", browserArgs(t, `{"ref":$0,"expression":"document === element"}`, root)), "value"), true)
	expectValue(t, observedField(t, f.command(t, tab, "eval", `{"css":".catalog-delete","all":true,"expression":"elements.map(element => element.textContent)"}`), "value"), []string{"Delete", "Delete"})
	f.rejects(t, tab, "eval", browserArgs(t, `{"ref":$0,"expression":"element.textContent='mutated'"}`, ref), "side_effect_rejected", "not_started")
	for _, invalid := range []string{`{"match":{"css":"button"},"or":[{"match":{"css":"button"}}]}`, `{"match":{"css":"button"},"bogus":true}`, `{"or":[]}`, `{"match":{"css":"button","role":"button"}}`} {
		f.rejects(t, tab, "find", browserArgs(t, `{"query":true,"body":$0}`, invalid), "invalid_input", "not_started")
	}
	for _, args := range []string{`{"role":"button","name-pattern":"["}`, `{"text-pattern":"["}`, `{"css":"["}`} {
		f.rejects(t, tab, "find", args, "invalid_input", "not_started")
	}
	html, err := contract.Decode[string](f.read(t, tab, "#named", "html"))
	if err != nil || !strings.HasPrefix(html, "<button") {
		t.Fatalf("%s %v", html, err)
	}
	f.command(t, tab, "wait", `{"load":"load"}`)
}

func TestDialogsRejectAbsentAndInapplicableActions(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	for _, name := range []string{"dialog.accept", "dialog.dismiss"} {
		f.rejects(t, tab, name, `{}`, "dialog_not_found", "not_started")
	}
	for _, kind := range []string{"alert", "confirm"} {
		f.rejects(t, tab, "click", browserArgs(t, `{"css":$0}`, "#"+kind+"-dialog"), "dialog_blocked", "unknown")
		expectValue(t, observedField(t, f.command(t, tab, "dialog.inspect", `{}`), "dialog", "type"), kind)
		f.rejects(t, tab, "dialog.accept", `{"text":"not a prompt"}`, "invalid_dialog_action", "not_started")
		name, outcome := "dialog.dismiss", "dismissed"
		if kind == "alert" {
			f.rejects(t, tab, "dialog.accept", `{}`, "invalid_dialog_action", "not_started")
		} else {
			name, outcome = "dialog.accept", "accepted"
		}
		result := f.command(t, tab, name, `{}`)
		expectValue(t, observedField(t, result, "type"), kind)
		expectValue(t, observedField(t, result, "outcome"), outcome)
	}
	dialog := observedField(t, f.command(t, tab, "dialog.inspect", `{}`), "dialog")
	if len(dialog) > 0 && string(dialog) != "null" {
		t.Fatal(string(dialog))
	}
}

func TestCatalogUntargetedKeysSelectionDragAndConsoleCursors(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	f.click(t, tab, "#select-middle")
	typed := f.command(t, tab, "type", `{"text":"XY"}`)
	if len(observedField(t, typed, "target", "ref")) == 0 {
		t.Fatal(string(typed))
	}
	expectValue(t, f.read(t, tab, "#text", "value"), "heXYo")
	f.command(t, tab, "key", `{"key":"ControlOrMeta+A"}`)
	f.command(t, tab, "type", `{"text":"hello"}`)
	f.command(t, tab, "select-text", `{"css":"#text","text":"ll"}`)
	f.command(t, tab, "type", `{"text":"XY"}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "heXYo")
	f.command(t, tab, "select-text", `{"css":"#text","text":"XY","cursor":"after"}`)
	f.command(t, tab, "type", `{"text":"!"}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "heXY!o")
	f.rejects(t, tab, "select-text", `{"css":"#repeated-text","text":"word"}`, "ambiguous_target", "not_started")
	f.command(t, tab, "select-text", `{"css":"#repeated-text","text":"word","prefix":"second ","suffix":"!"}`)
	expectValue(t, f.eval(t, tab, "getSelection().toString()"), "word")
	f.click(t, tab, "#focus-document")
	if result := f.command(t, tab, "key", `{"key":"Escape"}`); len(observedField(t, result, "target")) != 0 {
		t.Fatal(string(result))
	}
	f.command(t, tab, "key", `{"css":"#key-navigate","key":"Shift"}`)
	navigation := f.command(t, tab, "key", `{"key":"Enter","wait-url":"**/#key-focus"}`)
	expectValue(t, observedField(t, navigation, "url"), f.url+"/#key-focus")
	if len(observedField(t, navigation, "target", "ref")) == 0 {
		t.Fatal(string(navigation))
	}
	f.command(t, tab, "move", `{"css":"#drag-area"}`)
	points, err := contract.Decode[[]string](f.eval(t, tab, `(()=>{const r=document.querySelector('#drag-area').getBoundingClientRect();return [(r.x+10)+','+(r.y+10),(r.x+40)+','+(r.y+30)]})()`))
	if err != nil {
		t.Fatal(err)
	}
	f.command(t, tab, "drag", browserArgs(t, `{"point":$0,"modifier":["Shift"]}`, points))
	expectValue(t, f.eval(t, tab, `dragEvents[0].type==='mousedown'&&dragEvents[0].shift&&dragEvents.at(-1).buttons===0&&dragEvents.at(-1).shift`), true)
	f.click(t, tab, "#console-burst")
	logs := f.command(t, tab, "logs", `{"limit":2,"filter":"catalog"}`)
	expectValue(t, observedField(t, logs, "truncated"), true)
	expectValue(t, observedField(t, logs, "entries", "1", "level"), "error")
	if again := f.command(t, tab, "logs", `{"limit":2,"filter":"catalog"}`); string(again) != string(logs) {
		t.Fatalf("%s != %s", again, logs)
	}
	expectValue(t, observedField(t, f.command(t, tab, "logs", browserArgs(t, `{"after":$0}`, observedField(t, logs, "cursor"))), "entries"), []string{})
	f.rejects(t, tab, "logs", `{"after":"foreign:0"}`, "stale_cursor", "not_started")
}

func TestURLObservationDeliversInputAndObservesTransientMatches(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	f.command(t, tab, "wait", browserArgs(t, `{"url":$0}`, f.url+"/"))
	for i := 1; i <= 2; i++ {
		f.command(t, tab, "click", browserArgs(t, `{"css":"#no-nav","wait-url":$0}`, f.url+"/"))
		expectValue(t, f.eval(t, tab, "noNavClicks"), i)
	}
	for _, css := range []string{"#immediate", "#delayed"} {
		f.command(t, tab, "click", browserArgs(t, `{"css":$0,"wait-url":"**/destination"}`, css))
		expectValue(t, f.eval(t, tab, "location.pathname"), "/destination")
		f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url))
	}
	f.command(t, tab, "click", `{"css":"#transient","wait-url":"**/transient"}`)
	expectValue(t, f.eval(t, tab, "location.pathname"), "/returned")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url))
	details := f.rejects(t, tab, "click", `{"css":"#no-nav","wait-url":"**/never","timeout":700}`, "timeout", "completed")
	if details.Tab == nil || details.URL == nil {
		t.Fatal(details)
	}
	f.command(t, tab, "key", `{"css":"#hash","key":"Enter","wait-url":"**/#changed"}`)
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/a/b/c"))
	f.rejects(t, tab, "wait", `{"url":"**/a/*","timeout":100}`, "timeout", "not_started")
	f.command(t, tab, "wait", `{"url":"**/a/**"}`)
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/a/?"))
	f.command(t, tab, "wait", `{"url":"**/a/?"}`)
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url+"/a/x"))
	f.rejects(t, tab, "wait", `{"url":"**/a/?","timeout":100}`, "timeout", "not_started")
	f.command(t, tab, "goto", browserArgs(t, `{"url":$0}`, f.url))
	f.rejects(t, tab, "click", `{"css":"#stall","timeout":700}`, "timeout", "completed")
}

func TestCatalogCrossOriginFramesScopeFocusAndReferences(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "catalog-frame")
	f.command(t, tab, "wait", `{"load":"load"}`)
	outer := observedField(t, f.command(t, tab, "find", `{"css":"#cross-frame"}`), "matches", "0", "ref")
	input := observedField(t, f.command(t, tab, "find", browserArgs(t, `{"frame":[$0],"label":"Cross frame input"}`, outer)), "matches", "0", "ref")
	f.command(t, tab, "fill", browserArgs(t, `{"ref":$0,"text":"cross"}`, input))
	typed := f.command(t, tab, "type", `{"text":"-focus"}`)
	expectValue(t, observedField(t, typed, "target", "ref"), input)
	for _, args := range []string{browserArgs(t, `{"ref":$0,"property":"value"}`, input)} {
		expectValue(t, observedField(t, f.command(t, tab, "read", args), "value"), "cross-focus")
	}
	expectValue(t, observedField(t, f.command(t, tab, "eval", browserArgs(t, `{"ref":$0,"expression":"element.value"}`, input)), "value"), "cross-focus")
	f.command(t, tab, "click", browserArgs(t, `{"frame":[$0],"css":"#cross-button"}`, outer))
	expectValue(t, observedField(t, f.command(t, tab, "read", browserArgs(t, `{"frame":[$0],"css":"#cross-button","property":"text"}`, outer)), "value"), "Cross clicked")
	logging, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	for {
		logs, err := contract.List(observedField(t, f.command(t, tab, "logs", `{"filter":"cross-frame console","level":["info"]}`), "entries"), contract.Decode[json.RawMessage])
		if err != nil {
			t.Fatal(err)
		}
		if len(logs) == 1 {
			break
		}
		if err := logging.Err(); err != nil {
			t.Fatalf("child-frame console entry not recorded: %v", err)
		}
	}
	f.click(t, tab, "#toggle-frame")
	expectValue(t, observedField(t, f.command(t, tab, "read", browserArgs(t, `{"ref":$0,"property":"visible"}`, input)), "value"), false)
	f.command(t, tab, "wait", browserArgs(t, `{"ref":$0,"state":"hidden"}`, input))
	f.click(t, tab, "#toggle-frame")
	nested := observedField(t, f.command(t, tab, "find", browserArgs(t, `{"frame":[$0],"css":"#nested"}`, outer)), "matches", "0", "ref")
	f.command(t, tab, "fill", browserArgs(t, `{"frame":[$0,$1],"label":"Nested input","text":"nested"}`, outer, nested))
	query := `{"frame":{"frame":{"match":{"css":"#cross-frame"}},"match":{"css":"#nested"}},"match":{"label":"Nested input"}}`
	expectValue(t, observedField(t, f.command(t, tab, "find", browserArgs(t, `{"query":true,"body":$0}`, query)), "count"), 1)
	tree := f.command(t, tab, "inspect", browserArgs(t, `{"frame":[$0],"view":"dom"}`, outer))
	if !strings.Contains(string(tree), "cross-focus") || strings.Contains(string(tree), "Appointment date") {
		t.Fatal(string(tree))
	}
	if len(f.tabs(t)) != 1 {
		t.Fatal("frame became a tab")
	}
	f.click(t, tab, "#replace-frame")
	f.command(t, tab, "wait", `{"text-match":"Frame ready","exact":true}`)
	f.rejects(t, tab, "read", browserArgs(t, `{"ref":$0,"property":"value"}`, input), "stale_ref", "not_started")
	input = observedField(t, f.command(t, tab, "find", browserArgs(t, `{"frame":[$0],"label":"Cross frame input"}`, outer)), "matches", "0", "ref")
	f.command(t, tab, "reload", `{}`)
	f.rejects(t, tab, "read", browserArgs(t, `{"ref":$0,"property":"value"}`, input), "stale_ref", "not_started")
}

func TestFramePointerActionsWaitForCompositedScroll(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "catalog-frame")
	f.command(t, tab, "wait", `{"load":"load"}`)
	outer := observedField(t, f.command(t, tab, "find", `{"css":"#cross-frame"}`), "matches", "0", "ref")
	before := f.eval(t, tab, "[innerWidth,innerHeight,devicePixelRatio]")
	for count := 1; count <= 20; count++ {
		f.command(t, tab, "fill", browserArgs(t, `{"frame":[$0],"css":"#cross-input","text":$1}`, outer, strconv.Itoa(count)))
		f.command(t, tab, "click", browserArgs(t, `{"frame":[$0],"css":"#cross-button"}`, outer))
		expectValue(t, observedField(t, f.command(t, tab, "read", browserArgs(t, `{"frame":[$0],"css":"#cross-button","attribute":"data-clicks"}`, outer)), "value"), strconv.Itoa(count))
	}
	expectValue(t, f.eval(t, tab, "[innerWidth,innerHeight,devicePixelRatio]"), before)
}

func TestCatalogProbeAndServerRecordedNativeFormState(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "")
	f.command(t, tab, "move", `{"css":"#named"}`)
	xy, err := contract.Decode[string](f.eval(t, tab, `(()=>{const r=document.querySelector('#named').getBoundingClientRect();return (r.x+5)+','+(r.y+5)})()`))
	if err != nil {
		t.Fatal(err)
	}
	probe := f.command(t, tab, "probe", browserArgs(t, `{"xy":$0}`, xy))
	matches, err := contract.List(observedField(t, probe, "matches"), contract.Decode[json.RawMessage])
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, node := range matches {
		name, _ := contract.Decode[string](observedField(t, node, "name"))
		width, _ := contract.Decode[float64](observedField(t, node, "bounds", "width"))
		found = found || name == "Accessible label" && width > 0
	}
	if !found {
		t.Fatal(string(probe))
	}
	ordinary := f.command(t, tab, "probe", browserArgs(t, `{"xy":$0,"include-non-interactable":true}`, xy))
	all, err := contract.List(observedField(t, ordinary, "matches"), contract.Decode[json.RawMessage])
	if err != nil || len(all) <= len(matches) {
		t.Fatalf("%s %v", ordinary, err)
	}
	f.rejects(t, tab, "probe", `{"xy":"-1,0"}`, "invalid_input", "not_started")
	for _, date := range []string{"2026-10-01", "2026-10-02"} {
		expectValue(t, f.read(t, tab, "[name=confirm]", "checked"), false)
		f.command(t, tab, "fill", browserArgs(t, `{"label":"Submission date","text":$0}`, date))
		f.command(t, tab, "check", `{"css":"[name=confirm]","value":true}`)
		f.click(t, tab, "#submit-final")
	}
	expectValue(t, f.get(t, "/submitted-state"), []string{"/submit?date=2026-10-01&confirm=on", "/submit?date=2026-10-02&confirm=on"})
}

func TestViewportOverridesArePerTabAndSurviveScreenshots(t *testing.T) {
	f := chromeFixture(t)
	first, second := f.open(t, ""), f.open(t, "")
	for _, tab := range []browserop.TabID{first, second} {
		expectValue(t, f.eval(t, tab, "[innerWidth,innerHeight,devicePixelRatio]"), []int{1280, 720, 1})
	}
	f.command(t, first, "viewport.set", `{"width":390,"height":844}`)
	for _, test := range []struct {
		tab           browserop.TabID
		width, height int
	}{{first, 390, 844}, {second, 1280, 720}} {
		info := observedField(t, f.command(t, test.tab, "info", `{}`), "viewport")
		expectValue(t, observedField(t, info, "width"), test.width)
		expectValue(t, observedField(t, info, "height"), test.height)
		expectValue(t, f.eval(t, test.tab, "[innerWidth,innerHeight,devicePixelRatio]"), []int{test.width, test.height, 1})
		shot := f.command(t, test.tab, "screenshot", browserArgs(t, `{"output":$0}`, string(test.tab)+".png"))
		expectValue(t, observedField(t, shot, "width"), test.width)
		expectValue(t, observedField(t, shot, "height"), test.height)
		expectValue(t, observedField(t, f.command(t, test.tab, "info", `{}`), "viewport"), info)
	}
	f.command(t, second, "viewport.set", `{"width":640,"height":480}`)
	f.command(t, first, "viewport.reset", `{}`)
	expectValue(t, f.eval(t, first, "[innerWidth,innerHeight,devicePixelRatio]"), []int{1280, 720, 1})
	expectValue(t, f.eval(t, second, "[innerWidth,innerHeight,devicePixelRatio]"), []int{640, 480, 1})
}

func TestInputAndAnimationProbesCleanUpOnCancellationAndDialogs(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "#cancel-typing")
	job := f.start(t, "type", browserArgs(t, `{"tab":$0,"css":"#text","text":$1}`, tab, strings.Repeat("x", 10000)))
	f.get(t, "/wait-typing")
	job.cancel()
	requireCancelled(t, job.join(t))
	expectValue(t, f.eval(t, tab, `keys.length>0&&keys.filter(e=>e.type==='keydown').length===keys.filter(e=>e.type==='keyup').length`), true)
	f.rejects(t, tab, "key", `{"css":"#key-dialog","key":"Control+x"}`, "dialog_blocked", "")
	f.command(t, tab, "dialog.dismiss", `{}`)
	expectValue(t, f.eval(t, tab, `keys.filter(e=>e.key==='Control'&&e.type==='keydown').length===keys.filter(e=>e.key==='Control'&&e.type==='keyup').length`), true)
	f.rejects(t, tab, "click", `{"css":"#moving","timeout":5000}`, "not_actionable", "not_started")
	expectValue(t, f.eval(t, tab, `Object.getOwnPropertyNames(document.querySelector('#moving')).filter(key=>key.startsWith('probe_')).length`), 0)
}

func TestUnregisteredPopupSurvivesItsOpenerClosing(t *testing.T) {
	f := chromeFixture(t)
	id := f.open(t, "")
	tab := f.tab(t, id)
	env := f.environment(t)
	// Direct input avoids the command's popup reconciliation step.
	raw, err := browserop.ParseOperation("browser.click", []byte(browserArgs(t, `{"tab":$0,"css":"#open-popup"}`, id)))
	if err != nil {
		t.Fatal(err)
	}
	click, ok := raw.(*browserop.ClickInput)
	if !ok {
		t.Fatalf("%T", raw)
	}
	if _, err := page.Click(t.Context(), tab, *click, time.Now().Add(5*time.Second)); err != nil {
		t.Fatal(err)
	}
	if _, err := tab.CloseRequest(t.Context(), 5*time.Second); err != nil {
		t.Fatal(err)
	}
	listed, err := env.Tabs(t.Context(), 5*time.Second)
	if err != nil || len(listed) != 1 || listed[0].ID() == id {
		t.Fatalf("%v %v", listed, err)
	}
	expectValue(t, observedField(t, f.command(t, listed[0].ID(), "info", `{}`), "url"), "about:blank")
	again, err := env.Tabs(t.Context(), 5*time.Second)
	if err != nil || len(again) != 1 || again[0].ID() != listed[0].ID() {
		t.Fatalf("%v %v", again, err)
	}
}

func TestCatalogLoadWaitTracksOnlyCurrentDocument(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "load-replaced")
	for _, load := range []string{"commit", "domcontentloaded"} {
		f.command(t, tab, "wait", browserArgs(t, `{"load":$0}`, load))
	}
	f.mutate(t, tab, `(()=>{const ready=Object.getOwnPropertyDescriptor(Document.prototype,"readyState").get;Object.defineProperty(document,"readyState",{get(){fetch("/typing-started");return ready.call(this)}})})()`)
	job := f.start(t, "wait", browserArgs(t, `{"tab":$0,"load":"load"}`, tab))
	f.get(t, "/wait-typing")
	f.get(t, "/release-load")
	result := job.join(t)
	failure, err := browserop.DecodeFailureDocument(result.stderr)
	if err != nil || failure.Error.Code != "navigation_failed" {
		t.Fatalf("%s %v", result.stderr, err)
	}
	f.command(t, tab, "wait", `{"load":"load"}`)
}
