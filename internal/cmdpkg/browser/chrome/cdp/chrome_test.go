//go:build acceptance && !windows

package cdp_test

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/chromedp/cdproto/browser"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

// TestChromeAcceptance costs a Chrome launch (typically a few seconds). Only a
// real browser proves CDP attachment, independent debugging cleanup, OOPIF DOM
// ownership and native WebMCP; scripted sockets cannot prove those behaviors.
func TestChromeAcceptance(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to the pinned Chrome for Testing executable")
	}
	ctx := t.Context()
	command := exec.Command(
		executable,
		"--headless=new",
		"--remote-debugging-port=0",
		"--user-data-dir="+t.TempDir(),
		"--no-first-run",
		"--no-default-browser-check",
		"--use-mock-keychain",
		"--password-store=basic",
		"--enable-features=NetworkService,NetworkServiceInProcess",
		"--disable-background-networking",
		"--site-per-process",
		"about:blank",
	)
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	stderr, err := command.StderrPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	address := make(chan string, 1)
	finished := make(chan error, 1)
	go func() {
		scanner := bufio.NewScanner(stderr)
		for scanner.Scan() {
			if _, url, ok := strings.Cut(scanner.Text(), "DevTools listening on "); ok {
				select {
				case address <- url:
				default:
				}
			}
		}
		scanErr := scanner.Err()
		waitErr := command.Wait()
		if scanErr != nil {
			finished <- scanErr
		} else {
			finished <- waitErr
		}
	}()
	t.Cleanup(func() {
		// Kill the entire test-owned process group, including any renderer still
		// exiting after Browser.close; Wait reaps the parent and joins the pipe reader.
		_ = syscall.Kill(-command.Process.Pid, syscall.SIGKILL)
		<-finished
	})
	var url string
	select {
	case url = <-address:
	case err := <-finished:
		finished <- err
		t.Fatalf("Chrome exited before CDP: %v", err)
	}
	connection, err := cdp.Dial(ctx, url)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = browser.Close().Do(protocol.WithExecutor(ctx, connection))
		if err := connection.Close(ctx); err != nil {
			t.Error(err)
		}
	}()
	id, err := target.CreateTarget("about:blank").Do(protocol.WithExecutor(ctx, connection))
	if err != nil {
		t.Fatal(err)
	}
	session, err := connection.Attach(ctx, id)
	if err != nil {
		t.Fatal(err)
	}
	evaluate := func(script string) json.RawMessage {
		t.Helper()
		value, exception, err := runtime.Evaluate(script).
			WithAwaitPromise(true).
			WithReturnByValue(true).
			Do(protocol.WithExecutor(ctx, session))
		if err != nil || exception != nil {
			t.Fatalf("%s: %v %v", script, err, exception)
		}
		return json.RawMessage(value.Value)
	}
	if got := evaluate("21*2"); string(got) != "42" {
		t.Fatal(string(got))
	}
	if got, err := cdptest.EvaluateIn(ctx, url, id, "6*7"); err != nil || string(got) != "42" {
		t.Fatal(string(got), err)
	}
	fixture, err := os.ReadFile("testdata/webmcp.html")
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Log("fixture request", r.Host, r.URL.Path)
		w.Header().Set("Content-Type", "text/html")
		if r.URL.Path == "/webmcp" {
			_, _ = w.Write(fixture)
			return
		}
		if r.URL.Path == "/frame" {
			_, _ = fmt.Fprint(w, "<!doctype html><title>Child</title><button>Frame button</button>")
			return
		}
		_, _ = fmt.Fprintf(
			w,
			`<!doctype html><title>CDP fixture</title><iframe `+
				`src="http://localhost:%s/frame"></iframe><script>const worker=new `+
				`Worker(URL.createObjectURL(new Blob(['self.answer=42;postMessage(42)'],`+
				`{type:'text/javascript'})));window.ready=new Promise(resolve=>worker.onmessage=resolve);</script>`,
			strings.Split(r.Host, ":")[1],
		)
	}))
	defer server.Close()
	navigate := func(path string) {
		t.Helper()
		loaded, err := session.Subscribe("Page.loadEventFired")
		if err != nil {
			t.Fatal(err)
		}
		defer loaded.Close()
		if err := page.Enable().Do(protocol.WithExecutor(ctx, session)); err != nil {
			t.Fatal(err)
		}
		t.Log("navigating", server.URL+path)
		if _, _, failure, _, err := page.Navigate(server.URL + path).
			Do(protocol.WithExecutor(ctx, session)); err != nil ||
			failure != "" {
			t.Fatal(failure, err)
		}
		t.Log("navigation replied, waiting for load")
		if _, err := loaded.Next(ctx); err != nil {
			t.Fatal(err)
		}
	}
	attached, err := session.Subscribe("Target.attachedToTarget")
	if err != nil {
		t.Fatal(err)
	}
	defer attached.Close()
	navigate("/")
	evaluate("window.ready.then(()=>true)")
	// Page load and the worker message do not acknowledge the iframe's
	// flattened session. The router publishes attachment after registering it.
	for {
		event, err := attached.Next(ctx)
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := cdp.DecodeEvent(event)
		if err != nil {
			t.Fatal(err)
		}
		child, ok := decoded.(*target.EventAttachedToTarget)
		if ok && child.TargetInfo.Type == "iframe" {
			break
		}
	}
	frames, err := cdp.CaptureFrames(ctx, session)
	if err != nil {
		t.Fatal(err)
	}
	if len(frames.Frames) < 2 || len(frames.Documents) < 2 {
		t.Fatalf("OOPIF inventory: %d frames, %d renderer documents", len(frames.Frames), len(frames.Documents))
	}
	debug := cdp.StartDebug(ctx, url, id)
	defer func() {
		if err := debug.Close(ctx); err != nil {
			t.Error(err)
		}
	}()
	call := func(caller uint64, name string, args any) json.RawMessage {
		t.Helper()
		input, err := browserop.ParseInput(name, mustValue(t, args))
		if err != nil {
			t.Fatal(err)
		}
		operation := cdp.NewOperation(ctx, ctx, 10*time.Second)
		defer operation.Close()
		value, err := cdp.ExecuteCommand(ctx, operation, debug, caller, "t1", input)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		return value
	}
	rows, err := browserop.DecodeCDPTargetsResult(call(1, "cdp.targets", map[string]any{"tab": "t1"}))
	if err != nil {
		t.Fatal(err)
	}
	worker := ""
	for _, row := range rows.Targets {
		if row.Kind == "worker" {
			worker = row.ID
		}
	}
	if worker == "" {
		t.Fatalf("worker missing: %+v", rows)
	}
	sent, err := browserop.DecodeCDPSendResult(
		call(
			1,
			"cdp.send",
			map[string]any{
				"tab":    "t1",
				"method": "Runtime.evaluate",
				"params": `{"expression":"self.answer","returnByValue":true}`,
				"target": worker,
			},
		),
	)
	if err != nil || !strings.Contains(string(sent.Result), `"value":42`) {
		t.Fatal(string(sent.Result), err)
	}
	for _, method := range []string{"Browser.close", "Target.createTarget", "Page.close", "SystemInfo.getInfo"} {
		operation := cdp.NewOperation(ctx, ctx, time.Second)
		_, err := cdp.ExecuteCommand(
			ctx,
			operation,
			debug,
			1,
			"t1",
			&browserop.CDPSendInput{Tab: "t1", Method: method, Params: "{}"},
		)
		operation.Close()
		requireCode(t, err, "cdp_method_denied")
	}
	call(1, "cdp.send", map[string]any{"tab": "t1", "method": "Runtime.enable", "params": "{}"})
	initial, err := browserop.DecodeCDPEventsResult(call(1, "cdp.events", map[string]any{"tab": "t1"}))
	if err != nil {
		t.Fatal(err)
	}
	call(
		1,
		"cdp.send",
		map[string]any{
			"tab":    "t1",
			"method": "Runtime.evaluate",
			"params": `{"expression":"console.log('first'); console.log('second')"}`,
		},
	)
	first, err := browserop.DecodeCDPEventsResult(
		call(
			1,
			"cdp.events",
			map[string]any{
				"tab":    "t1",
				"after":  initial.Cursor,
				"method": []string{"Runtime.consoleAPICalled"},
				"limit":  1,
			},
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(first.Events) != 1 || !first.HasMore {
		t.Fatalf("first page: %+v", first)
	}
	second, err := browserop.DecodeCDPEventsResult(
		call(
			1,
			"cdp.events",
			map[string]any{
				"tab":    "t1",
				"after":  first.Cursor,
				"method": []string{"Runtime.consoleAPICalled"},
				"limit":  1,
			},
		),
	)
	if err != nil || len(second.Events) != 1 || second.HasMore {
		t.Fatal(second, err)
	}
	call(2, "cdp.send", map[string]any{"tab": "t1", "method": "Runtime.enable", "params": "{}"})
	call(1, "cdp.detach", map[string]any{"tab": "t1"})
	callers := debug.OtherCallers(nil)
	if len(callers) != 1 || callers[0] != 2 {
		t.Fatal(callers)
	}
	call(2, "cdp.detach", map[string]any{"tab": "t1"})
	navigate("/webmcp")
	state := &cdp.WebMCPState{}
	capabilities, err := cdp.Capabilities(ctx, session)
	if err != nil {
		t.Fatal(err)
	}
	if !capabilities[0].Available {
		t.Fatalf("pinned Chrome has no native WebMCP: %v", capabilities[0].Reason)
	}
	web := func(command browserop.Operation) (json.RawMessage, error) {
		operation := cdp.NewOperation(ctx, ctx, 10*time.Second)
		defer operation.Close()
		return cdp.ExecuteWebMCP(ctx, operation, session, state, "t1", command)
	}
	raw, err := web(&browserop.WebMCPListInput{Tab: "t1"})
	if err != nil {
		t.Fatal(err)
	}
	tools, err := browserop.DecodeWebMCPListResult(raw)
	if err != nil {
		t.Fatal(err)
	}
	raw, err = web(
		&browserop.WebMCPCallInput{Tab: "t1", Tools: tools.Tools, Tool: "echo", Arguments: `{"text":"hello"}`},
	)
	if err != nil {
		t.Fatal(err)
	}
	reply, err := browserop.DecodeWebMCPCallResult(raw)
	if err != nil || string(reply.Result) != `{"echo":"hello"}` {
		t.Fatal(string(raw), err)
	}
	_, err = web(&browserop.WebMCPCallInput{Tab: "t1", Tools: tools.Tools, Tool: "echo", Arguments: `{"text":42}`})
	requireCode(t, err, "invalid_input")
	evaluate("document.querySelector('#replace').onclick().then(()=>true)")
	_, err = web(&browserop.WebMCPCallInput{Tab: "t1", Tools: tools.Tools, Tool: "echo", Arguments: `{"text":"again"}`})
	requireCode(t, err, "stale_tools")
}
