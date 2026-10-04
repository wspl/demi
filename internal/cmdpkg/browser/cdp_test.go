//go:build acceptance

package browser

import (
	"encoding/json"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/contract"
)

func TestCDPValidatesMethodsScopesChildrenAndPreservesEventCursors(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "cdp.html")
	f.command(t, tab, "wait", `{"css":"#worker-ready","state":"attached"}`)
	for _, method := range []string{"Browser.close", "Target.createTarget", "Page.close", "SystemInfo.getInfo"} {
		f.rejects(t, tab, "cdp.send", browserArgs(t, `{"method":$0,"params":"{}"}`, method), "cdp_method_denied", "")
	}
	for _, test := range []struct{ method, params string }{{"Missing.command", "{}"}, {"Runtime.evaluate", "{}"}, {"Runtime.evaluate", `{"expression":1}`}, {"Runtime.evaluate", `{"expression":"1","sessionId":"external"}`}} {
		completion, _, stderr := f.result(
			t,
			"cdp.send",
			browserArgs(t, `{"tab":$0,"method":$1,"params":$2}`, tab, test.method, test.params),
		)
		if completion.ExitCode != 2 {
			t.Fatalf("%+v %s", completion, stderr)
		}
	}
	targets := f.command(t, tab, "cdp.targets", `{}`)
	rows, err := contract.List(observedField(t, targets, "targets"), contract.Decode[json.RawMessage])
	if err != nil {
		t.Fatal(err)
	}
	var worker json.RawMessage
	main := false
	for _, row := range rows {
		main = main || string(observedField(t, row, "id")) == `"main"`
		if string(observedField(t, row, "kind")) == `"worker"` {
			worker = observedField(t, row, "id")
		}
	}
	if !main || worker == nil {
		t.Fatal(string(targets))
	}
	value := f.command(
		t,
		tab,
		"cdp.send",
		browserArgs(
			t,
			`{"method":"Runtime.evaluate","params":$0,"target":$1}`,
			`{"expression":"self.answer","returnByValue":true}`,
			worker,
		),
	)
	expectValue(t, observedField(t, value, "result", "result", "value"), 42)
	f.command(t, tab, "cdp.send", `{"method":"Runtime.enable","params":"{}"}`)
	initial := f.command(t, tab, "cdp.events", `{"method":["Runtime.consoleAPICalled"]}`)
	expectValue(t, observedField(t, initial, "events"), []string{})
	f.mutate(t, tab, `console.log('first'); console.log('second')`)
	first := f.command(
		t,
		tab,
		"cdp.events",
		browserArgs(
			t,
			`{"method":["Runtime.consoleAPICalled"],"after":$0,"limit":1}`,
			observedField(t, initial, "cursor"),
		),
	)
	expectValue(t, observedField(t, first, "hasMore"), true)
	second := f.command(
		t,
		tab,
		"cdp.events",
		browserArgs(
			t,
			`{"method":["Runtime.consoleAPICalled"],"after":$0,"limit":1}`,
			observedField(t, first, "cursor"),
		),
	)
	expectValue(t, observedField(t, second, "hasMore"), false)
	for _, result := range [][]byte{first, second} {
		rows, err := contract.List(observedField(t, result, "events"), contract.Decode[json.RawMessage])
		if err != nil || len(rows) != 1 {
			t.Fatalf("events=%s: %v", result, err)
		}
	}
	a, err := contract.Decode[uint64](observedField(t, first, "events", "0", "sequence"))
	if err != nil {
		t.Fatal(err)
	}
	b, err := contract.Decode[uint64](observedField(t, second, "events", "0", "sequence"))
	if err != nil || b <= a {
		t.Fatalf("sequence %d %d %v", a, b, err)
	}
	f.rejects(t, tab, "cdp.events", `{"after":"expired:0"}`, "stale_cursor", "")
	f.rejects(
		t,
		tab,
		"cdp.send",
		browserArgs(t, `{"method":"Runtime.evaluate","params":$0,"target":"external"}`, `{"expression":"1"}`),
		"target_not_found",
		"",
	)
}

func TestCDPWaitExpiryRetainsSubscriptionsAndInvocationCancellationReleasesConnections(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "cdp.html")
	f.command(t, tab, "cdp.send", `{"method":"Runtime.enable","params":"{}"}`)
	before := observedField(t, f.command(t, tab, "cdp.events", `{}`), "cursor")
	empty := f.command(
		t,
		tab,
		"cdp.events",
		browserArgs(t, `{"after":$0,"method":["Runtime.consoleAPICalled"],"timeout":100}`, before),
	)
	expectValue(t, observedField(t, empty, "events"), []string{})
	f.mutate(t, tab, `console.log('retained')`)
	events := f.command(
		t,
		tab,
		"cdp.events",
		browserArgs(t, `{"after":$0,"method":["Runtime.consoleAPICalled"]}`, before),
	)
	rows, err := contract.List(observedField(t, events, "events"), contract.Decode[json.RawMessage])
	if err != nil || len(rows) != 1 {
		t.Fatalf("%s %v", events, err)
	}
	job := f.start(
		t,
		"cdp.events",
		browserArgs(
			t,
			`{"tab":$0,"after":$1,"method":["Runtime.consoleAPICalled"],"timeout":30000}`,
			tab,
			observedField(t, events, "cursor"),
		),
	)
	f.waitBusy(t, tab)
	select {
	case result := <-job.done:
		job.done <- result
		t.Fatalf("command completed before cancellation: %+v", result)
	default:
	}
	job.cancel()
	requireCancelled(t, job.join(t))
	f.rejects(t, tab, "cdp.events", browserArgs(t, `{"after":$0}`, before), "stale_cursor", "")
	for _, method := range []string{"Debugger.enable", "Fetch.enable"} {
		f.command(t, tab, "cdp.send", browserArgs(t, `{"method":$0,"params":"{}"}`, method))
	}
	paused := f.start(
		t,
		"cdp.send",
		browserArgs(
			t,
			`{"tab":$0,"method":"Runtime.evaluate","params":$1,"timeout":30000}`,
			tab,
			`{"expression":"debugger;42","returnByValue":true}`,
		),
	)
	f.waitBusy(t, tab)
	select {
	case result := <-paused.done:
		paused.done <- result
		t.Fatalf("debugger command completed before cancellation: %+v", result)
	default:
	}
	paused.cancel()
	requireCancelled(t, paused.join(t))
	expectValue(t, f.eval(t, tab, "21*2"), 42)
	f.rejects(t, tab, "cdp.events", browserArgs(t, `{"after":$0}`, before), "stale_cursor", "")
	f.command(t, tab, "goto", `{"url":"about:blank","timeout":3000}`)
}

func TestCDPDetachReleasesOnlyItsCallerAndTimeoutsIdentifyOtherDebugOwners(t *testing.T) {
	first := chromeFixture(t)
	first.caller = 1
	second := *first
	second.caller = 2
	tab := first.open(t, "cdp.html")
	first.command(t, tab, "cdp.send", `{"method":"Fetch.enable","params":"{}"}`)
	second.command(t, tab, "cdp.send", `{"method":"Runtime.enable","params":"{}"}`)
	before := observedField(t, second.command(t, tab, "cdp.events", `{}`), "cursor")
	failure := second.failure(t, "goto", browserArgs(t, `{"tab":$0,"url":$1,"timeout":300}`, tab, first.url), "timeout")
	if failure.Error.Details == nil || failure.Error.Details.DebuggingCallers == nil {
		t.Fatal(failure)
	}
	expectValue(t, mustBrowserValue(t, *failure.Error.Details.DebuggingCallers), []uint64{1})
	if failure.Error.Details.Tab == nil || *failure.Error.Details.Tab != string(tab) {
		t.Fatal(failure)
	}
	detached := first.command(t, tab, "cdp.detach", `{}`)
	expectValue(t, observedField(t, detached, "detached"), tab)
	expectValue(t, first.command(t, tab, "cdp.detach", `{}`), json.RawMessage(detached))
	second.command(t, tab, "goto", browserArgs(t, `{"url":$0,"timeout":3000}`, first.url))
	second.mutate(t, tab, `console.log('still subscribed')`)
	events := second.command(
		t,
		tab,
		"cdp.events",
		browserArgs(t, `{"after":$0,"method":["Runtime.consoleAPICalled"]}`, before),
	)
	rows, err := contract.List(observedField(t, events, "events"), contract.Decode[json.RawMessage])
	if err != nil || len(rows) != 1 {
		t.Fatalf("%s %v", events, err)
	}
	timeout := second.failure(
		t,
		"goto",
		browserArgs(t, `{"tab":$0,"url":$1,"timeout":300}`, tab, first.url+"/stall"),
		"timeout",
	)
	if timeout.Error.Details != nil && timeout.Error.Details.DebuggingCallers != nil {
		t.Fatal(timeout)
	}
}

func TestTabCloseJoinsPausedDebugConnectionsAndPreservesOtherTabs(t *testing.T) {
	f := chromeFixture(t)
	tab, other := f.open(t, "cdp.html"), f.open(t, "cdp.html")
	f.command(t, tab, "cdp.send", `{"method":"Debugger.enable","params":"{}"}`)
	job := f.start(
		t,
		"cdp.send",
		browserArgs(
			t,
			`{"tab":$0,"method":"Runtime.evaluate","params":$1,"timeout":30000}`,
			tab,
			`{"expression":"debugger;42"}`,
		),
	)
	f.waitBusy(t, tab)
	select {
	case result := <-job.done:
		job.done <- result
		t.Fatalf("debugger command completed before close: %+v", result)
	default:
	}
	f.command(t, tab, "close", `{}`)
	result := job.join(t)
	failure, err := browserproto.DecodeFailureDocument(result.stderr)
	if err != nil || result.completion.ExitCode == 0 ||
		failure.Error.Code != "browser_lost" && failure.Error.Code != "tab_not_found" {
		t.Fatalf("%+v %s %v", result, result.stderr, err)
	}
	f.rejects(t, tab, "cdp.detach", `{}`, "tab_not_found", "")
	f.command(t, other, "goto", `{"url":"about:blank"}`)
}

func TestElementWaitAndInputResampleNodesInsertedDuringLocatorResolution(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "cdp.html")
	for _, state := range []string{"attached", "hidden", "click"} {
		selector := "#inserted-" + state
		script := `(()=>{const original=Element.prototype.matches; Element.prototype.matches=function(selector){if(selector===` + string(
			mustBrowserValue(t, selector),
		) + `&&this===document.documentElement){const node=document.createElement('button');node.id=selector.slice(1);node.textContent='Inserted during lookup';node.onclick=()=>document.body.dataset.insertedClicked='yes';document.documentElement.append(node);Element.prototype.matches=original;}return original.call(this,selector)}})()`
		f.mutate(t, tab, script)
		if state == "click" {
			f.click(t, tab, selector)
			expectValue(t, f.eval(t, tab, "document.body.dataset.insertedClicked"), "yes")
			continue
		}
		args := browserArgs(t, `{"css":$0,"state":$1,"timeout":500}`, selector, state)
		if state == "hidden" {
			f.rejects(t, tab, "wait", args, "timeout", "")
		} else {
			result := f.command(t, tab, "wait", args)
			expectValue(t, observedField(t, result, "matched"), true)
			if _, err := contract.Decode[string](observedField(t, result, "target", "ref")); err != nil {
				t.Fatal(string(result))
			}
		}
	}
}

func TestOversizedObservationEndsBrowserAndAllowsFreshOpen(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "cdp.html")
	f.mutate(t, tab, `document.body.innerHTML='<button>Large observation</button>'.repeat(20000);undefined`)
	f.rejects(t, tab, "inspect", `{}`, "browser_lost", "")
	// Each attempt is a real open; only browser_lost, the retiring browser, repeats.
	for {
		completion, stdout, stderr := f.result(t, "open", `{"url":"about:blank"}`)
		if completion.ExitCode == 0 {
			opened, err := browserproto.DecodeOpenResult(stdout)
			if err != nil || opened.Tab == tab {
				t.Fatalf("%s %v", stdout, err)
			}
			return
		}
		failure, err := browserproto.DecodeFailureDocument(stderr)
		if err != nil || failure.Error.Code != "browser_lost" {
			t.Fatalf("%s %v", stderr, err)
		}
	}
}

func TestCDPEvictionMarksTruncationAndWorkerHandlesExpire(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "cdp.html")
	f.command(t, tab, "wait", `{"css":"#worker-ready","state":"attached"}`)
	targets := f.command(t, tab, "cdp.targets", `{}`)
	rows, err := contract.List(observedField(t, targets, "targets"), contract.Decode[json.RawMessage])
	if err != nil {
		t.Fatal(err)
	}
	var worker json.RawMessage
	for _, row := range rows {
		if string(observedField(t, row, "kind")) == `"worker"` {
			worker = observedField(t, row, "id")
		}
	}
	if worker == nil {
		t.Fatal(string(targets))
	}
	f.command(
		t,
		tab,
		"cdp.send",
		browserArgs(
			t,
			`{"method":"Runtime.evaluate","params":$0}`,
			`{"expression":"window.open('about:blank');true","userGesture":true}`,
		),
	)
	scoped, err := contract.List(
		observedField(t, f.command(t, tab, "cdp.targets", `{}`), "targets"),
		contract.Decode[json.RawMessage],
	)
	if err != nil || len(scoped) != 2 {
		t.Fatalf("%s %v", scoped, err)
	}
	paged := f.command(t, tab, "cdp.targets", `{"offset":1,"limit":1}`)
	expectValue(t, observedField(t, paged, "truncated"), false)
	pageRows, err := contract.List(observedField(t, paged, "targets"), contract.Decode[json.RawMessage])
	if err != nil || len(pageRows) != 1 {
		t.Fatalf("%s: %v", paged, err)
	}
	f.command(t, tab, "cdp.send", `{"method":"Runtime.enable","params":"{}"}`)
	initial := observedField(t, f.command(t, tab, "cdp.events", `{}`), "cursor")
	f.mutate(t, tab, `for(let n=0;n<10020;n++)console.log(n);worker.terminate()`)
	events := f.command(
		t,
		tab,
		"cdp.events",
		browserArgs(t, `{"after":$0,"method":["Runtime.consoleAPICalled"],"limit":1}`, initial),
	)
	expectValue(t, observedField(t, events, "truncated"), true)
	expectValue(t, observedField(t, events, "hasMore"), true)
	// Each attempt is a real target listing; only a still-listed worker repeats.
	for {
		targets := f.command(t, tab, "cdp.targets", `{}`)
		rows, err := contract.List(observedField(t, targets, "targets"), contract.Decode[json.RawMessage])
		if err != nil {
			t.Fatal(err)
		}
		found := false
		for _, row := range rows {
			found = found || string(observedField(t, row, "id")) == string(worker)
		}
		if !found {
			break
		}
	}
	f.rejects(
		t,
		tab,
		"cdp.send",
		browserArgs(t, `{"target":$0,"method":"Runtime.evaluate","params":$1}`, worker, `{"expression":"1"}`),
		"target_not_found",
		"",
	)
	f.command(t, tab, "goto", `{"url":"about:blank"}`)
	expectValue(t, observedField(t, f.command(t, tab, "cdp.targets", `{}`), "targets", "0", "url"), "about:blank")
}
