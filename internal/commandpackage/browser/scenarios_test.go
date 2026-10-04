//go:build acceptance

package browser

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/contract"
)

// browserSite serves testdata/repairs.html and its navigation, form and cancellation endpoints.
func browserSite(t *testing.T) http.Handler {
	t.Helper()
	fixture, err := os.ReadFile("testdata/repairs.html")
	if err != nil {
		t.Fatal(err)
	}
	var reloads, effects atomic.Uint32
	var mu sync.Mutex
	submissions := []string{}
	typing, replacement := make(chan struct{}), make(chan struct{})
	var typed, replaced sync.Once
	files := http.FileServer(http.Dir("testdata"))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/side-effect":
			effects.Add(1)
			return
		case "/effects-count":
			_, _ = io.WriteString(w, strconv.FormatUint(uint64(effects.Load()), 10))
			return
		case "/drop":
			panic(http.ErrAbortHandler)
		case "/reload-drop":
			if reloads.Add(1) > 1 {
				panic(http.ErrAbortHandler)
			}
		case "/stall":
			<-r.Context().Done()
			return
		case "/stream-download":
			typed.Do(func() {
				close(typing)
			})
			w.Header().Set("Content-Disposition", "attachment; filename=stream.bin")
			w.Header().Set("Content-Length", "10485760")
			w.Header().Set("Content-Type", "application/octet-stream")
			if _, err := w.Write([]byte(strings.Repeat("x", 1024))); err != nil {
				return
			}
			if err := http.NewResponseController(w).Flush(); err != nil {
				return
			}
			<-r.Context().Done()
			return
		case "/typing-started":
			typed.Do(func() {
				close(typing)
			})
		case "/wait-typing":
			select {
			case <-typing:
			case <-r.Context().Done():
				return
			}
		case "/release-load":
			replaced.Do(func() {
				close(replacement)
			})
		case "/replace-load":
			select {
			case <-replacement:
			case <-r.Context().Done():
				return
			}
		case "/submitted-state":
			mu.Lock()
			data, err := contract.EncodeJSON(submissions)
			mu.Unlock()
			if err != nil {
				t.Error(err)
				return
			}
			_, _ = w.Write(data)
			return
		case "/submit":
			mu.Lock()
			submissions = append(submissions, r.URL.RequestURI())
			mu.Unlock()
		case "/load-replaced":
			_, _ = io.WriteString(
				w,
				`<!doctype html><img src='/stall'><script>fetch('/replace-load').then(() => location.href='/')</script>`,
			)
			return
		case "/500":
			w.WriteHeader(500)
			_, _ = io.WriteString(w, `<!doctype html><title>HTTP error document</title><h1>Received 500</h1>`)
			return
		case "/redirect":
			http.Redirect(w, r, "/destination", http.StatusFound)
			return
		case "/destination":
			_, _ = io.WriteString(w, `<!doctype html><title>Destination</title><h1>Arrived</h1>`)
			return
		case "/frame-child":
			_, _ = io.WriteString(
				w,
				`<!doctype html><label>Cross frame input<input id='cross-input'></label><button id='cross-button' onclick='this.textContent="Cross clicked";this.dataset.clicks=String(Number(this.dataset.clicks||0)+1);console.info("cross-frame console")'>Cross button</button><iframe id='nested' srcdoc="<label>Nested input<input id='nested-input'></label>"></iframe><script>window.pointerEvents=[];for(const type of ['pointerdown','mousedown','pointerup','mouseup','click','mousemove','scroll','focusin'])document.addEventListener(type,event=>pointerEvents.push({type,target:event.target.id,x:event.clientX,y:event.clientY,scrollY,top:document.querySelector('#cross-button').getBoundingClientRect().top,at:Date.now()}),true);</script>`,
			)
			return
		}
		if filepath.Ext(r.URL.Path) != "" {
			files.ServeHTTP(w, r)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(fixture)
	})
}

// observedField selects browser result values for wire assertions without declaring a second result shape.
func observedField(t *testing.T, raw []byte, path ...string) json.RawMessage {
	t.Helper()
	for _, key := range path {
		if index, err := strconv.Atoi(key); err == nil {
			values, err := contract.List(raw, contract.Decode[json.RawMessage])
			if err != nil || index >= len(values) {
				t.Fatalf("field %s in %s: %v", key, raw, err)
			}
			raw = values[index]
		} else {
			fields, err := contract.ObjectFields(raw)
			if err != nil {
				t.Fatal(err)
			}
			found := false
			for _, field := range fields {
				if field.Name == key {
					value, ok := field.Value.(json.RawMessage)
					if !ok {
						t.Fatalf("field %s is %T", key, field.Value)
					}
					raw = value
					found = true
					break
				}
			}
			if !found {
				return nil
			}
		}
	}
	return raw
}

func expectValue(t *testing.T, got []byte, expected any) {
	t.Helper()
	want, err := contract.EncodeJSON(expected)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(got)) != string(want) {
		t.Fatalf("got %s, want %s", got, want)
	}
}

func (f *browserFixture) command(t *testing.T, tab browserproto.TabID, name, args string) []byte {
	t.Helper()
	if args == "" {
		args = `{}`
	}
	args = `{"tab":` + string(mustBrowserValue(t, tab)) + `,` + strings.TrimPrefix(args, "{")
	args = strings.ReplaceAll(args, ",}", "}")
	return f.call(t, name, args)
}

func mustBrowserValue(t *testing.T, v any) []byte {
	t.Helper()
	raw, err := cdp.Value(v)
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func (f *browserFixture) eval(t *testing.T, tab browserproto.TabID, expression string) json.RawMessage {
	t.Helper()
	result, err := browserproto.DecodeEvalResult(
		f.command(t, tab, "eval", browserArgs(t, `{"expression":$0}`, expression)),
	)
	if err != nil {
		t.Fatal(err)
	}
	return result.Value
}

func (f *browserFixture) click(t *testing.T, tab browserproto.TabID, css string) {
	t.Helper()
	f.command(t, tab, "click", browserArgs(t, `{"css":$0}`, css))
}

func (f *browserFixture) rejects(
	t *testing.T,
	tab browserproto.TabID,
	name, args, code, action string,
) browserproto.ErrorDetails {
	t.Helper()
	args = `{"tab":` + string(mustBrowserValue(t, tab)) + `,` + strings.TrimPrefix(args, "{")
	args = strings.ReplaceAll(args, ",}", "}")
	failure := f.failure(t, name, args, code)
	if failure.Error.Details == nil {
		t.Fatal("missing failure details")
	}
	details := *failure.Error.Details
	if action != "" && (details.Action == nil || string(*details.Action) != action) {
		t.Fatalf("%s: action=%v want %s, details=%s", name, *details.Action, action, mustBrowserValue(t, details))
	}
	return details
}

func (f *browserFixture) tab(t *testing.T, id browserproto.TabID) *tabs.Tab {
	t.Helper()
	f.s.mu.Lock()
	b := f.s.browsers[f.conversation]
	f.s.mu.Unlock()
	environment, _, err := b.Running(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	tab, err := environment.Tab(t.Context(), id, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	return tab
}

// waitingCommand owns cancellation and collection, including failure cleanup.
type waitingCommand struct {
	cancel context.CancelFunc
	done   chan commandAnswer
}
type commandAnswer struct {
	completion     commandproto.Completion
	stdout, stderr []byte
	err            error
}

func (f *browserFixture) start(t *testing.T, name, args string) *waitingCommand {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	request := invocation(name, args, f.conversation)
	request.Request.Cwd = f.root
	if f.caller != 0 {
		request.Request.Context.Caller = &commandproto.AgentCaller{Number: f.caller}
	}
	if f.locale != nil {
		request.Request.Context.Locale = *f.locale
	}
	output, records := commandsdk.OutputChannel(ctx)
	request.Output = output
	job := &waitingCommand{cancel: cancel, done: make(chan commandAnswer, 1)}
	go func() {
		for {
			c, out, errout, err := collectInvocation(ctx, f.s, request, records)
			failure, decodeErr := browserproto.DecodeFailureDocument(errout)
			if err == nil && decodeErr == nil && failure.Error.Code == "tab_busy" {
				// Admission polling can win the gate. Retry only that refused test start.
				runtime.Gosched()
				continue
			}
			job.done <- commandAnswer{c, out, errout, err}
			return
		}
	}()
	t.Cleanup(func() {
		cancel()
		result := <-job.done
		job.done <- result
	})
	return job
}

func (w *waitingCommand) join(t *testing.T) commandAnswer {
	t.Helper()
	r := <-w.done
	w.done <- r
	return r
}

func (f *browserFixture) waitBusy(t *testing.T, id browserproto.TabID) {
	t.Helper()
	tab := f.tab(t, id)
	// The gate offers no admission event; each attempt is a real checkout.
	for {
		checkout := tab.Gate().TryCheckout()
		if checkout == nil {
			return
		}
		checkout.Release()
		runtime.Gosched()
		select {
		case <-tab.Done():
			t.Fatal("tab ended before command admission")
		default:
		}
	}
}

func requireCancelled(t *testing.T, r commandAnswer) {
	t.Helper()
	if errors.Is(r.err, context.Canceled) {
		return
	}
	failure, err := browserproto.DecodeFailureDocument(r.stderr)
	if r.err != nil || r.completion.ExitCode != 130 || err != nil || failure.Error.Code != "cancelled" {
		t.Fatalf("cancel: %+v %s: %v", r, r.stderr, err)
	}
}

func (f *browserFixture) mutate(t *testing.T, tab browserproto.TabID, expression string) []byte {
	t.Helper()
	params := string(mustBrowserValue(t, struct {
		Expression    string `json:"expression"`
		ReturnByValue bool   `json:"returnByValue"`
		AwaitPromise  bool   `json:"awaitPromise"`
	}{expression, true, true}))
	return f.command(t, tab, "cdp.send", browserArgs(t, `{"method":"Runtime.evaluate","params":$0}`, params))
}

// get reads the fixture server's synchronization and recorded submission endpoints.
func (f *browserFixture) get(t *testing.T, path string) []byte {
	t.Helper()
	request, err := http.NewRequestWithContext(t.Context(), http.MethodGet, f.url+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			t.Error(err)
		}
	}()
	data, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// fixtureInput supplies framed viewer messages or finite clipboard bytes.
type fixtureInput struct{ data chan []byte }

// Next waits for fixture input or cancellation.
func (f *fixtureInput) Next(ctx context.Context) ([]byte, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case data, ok := <-f.data:
		if !ok {
			return nil, io.EOF
		}
		return data, nil
	}
}

// environment observes the conversation's published browser, never starts one.
func (f *browserFixture) environment(t *testing.T) *tabs.Environment {
	t.Helper()
	f.s.mu.Lock()
	owner := f.s.browsers[f.conversation]
	f.s.mu.Unlock()
	env, _, err := owner.Running(t.Context())
	if err != nil || env == nil {
		t.Fatalf("environment %p %v", env, err)
	}
	return env
}
