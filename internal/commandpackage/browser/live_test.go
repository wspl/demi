//go:build acceptance

package browser

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"runtime"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/contract"
)

type browserView struct {
	ctx        context.Context
	cancel     context.CancelFunc
	input      *fixtureInput
	records    <-chan commandproto.Record
	done       chan commandAnswer
	pending    []byte
	closeInput sync.Once
}

type viewFrame struct {
	control json.RawMessage
	header  browserproto.VideoHeader
	video   []byte
}

// view runs the service's actual live entry point and owns its stream and join.
func (f *browserFixture) view(t *testing.T) *browserView {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	v := &browserView{
		ctx:    ctx,
		cancel: cancel,
		input:  &fixtureInput{data: make(chan []byte, 32)},
		done:   make(chan commandAnswer, 1),
	}
	output, records := commandsdk.OutputChannel(ctx)
	v.records = records
	request := invocation("live", `{}`, "acceptance")
	request.Request.Context.Caller = &commandproto.UserCaller{}
	request.Input = commandsdk.NewInput(v.input)
	request.Output = output
	go func() {
		completion, err := f.s.Invoke(ctx, request)
		v.done <- commandAnswer{completion: completion, err: err}
	}()
	t.Cleanup(func() {
		cancel()
		<-v.done
	})
	return v
}

func (v *browserView) send(t *testing.T, message browserproto.LiveViewerMessage) {
	t.Helper()
	data, err := (browserproto.LiveViewerMessageJSON{Value: message}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	frame := binary.BigEndian.AppendUint32(nil, uint32(len(data)+1))
	frame = append(frame, browserproto.ControlFrame)
	frame = append(frame, data...)
	select {
	case v.input.data <- frame:
	case <-v.ctx.Done():
		t.Fatal(v.ctx.Err())
	}
}

func (v *browserView) next(t *testing.T) viewFrame {
	t.Helper()
	for {
		if len(v.pending) >= 4 {
			n := int(binary.BigEndian.Uint32(v.pending))
			if n < 1 || n > browserproto.MaxFrameBytes {
				t.Fatalf("invalid live frame size %d", n)
			}
			if len(v.pending) >= 4+n {
				frame := bytes.Clone(v.pending[4 : 4+n])
				v.pending = v.pending[4+n:]
				switch frame[0] {
				case browserproto.ControlFrame:
					_, err := browserproto.DecodeLiveModuleMessage(frame[1:])
					if err != nil {
						t.Fatal(err)
					}
					// Recovery notices are control frames the test can observe.
					// Waiting for the requested stream decides whether recovery succeeded.
					return viewFrame{control: frame[1:]}
				case browserproto.VideoFrame:
					header, payload, err := browserproto.SplitVideoFrame(frame[1:])
					if err != nil {
						t.Fatal(err)
					}
					v.send(
						t,
						&browserproto.LiveViewerMessageAck{Generation: header.Generation, Sequence: header.Sequence},
					)
					return viewFrame{header: header, video: payload}
				default:
					t.Fatalf("unknown frame %d", frame[0])
				}
			}
		}
		select {
		case record := <-v.records:
			stdout, ok := record.(commandproto.Stdout)
			if !ok {
				t.Fatalf("live output %T", record)
			}
			v.pending = append(v.pending, stdout...)
		case <-v.ctx.Done():
			t.Fatal(v.ctx.Err())
		}
	}
}

func (v *browserView) until(t *testing.T, kind string, match func(json.RawMessage) bool) json.RawMessage {
	t.Helper()
	for {
		frame := v.next(t)
		if frame.control != nil && string(observedField(t, frame.control, "type")) == `"`+kind+`"` &&
			(match == nil || match(frame.control)) {
			return frame.control
		}
	}
}

func (v *browserView) hello(t *testing.T, platform browserproto.Platform) {
	v.send(t, &browserproto.LiveViewerMessageHello{Platform: platform})
	v.send(
		t,
		&browserproto.LiveViewerMessagePanel{
			Width:            800,
			Height:           600,
			DevicePixelRatio: 2,
			ScreenWidth:      1440,
			ScreenHeight:     900,
		},
	)
}

func (v *browserView) watch(t *testing.T, tab browserproto.TabID) {
	t.Helper()
	v.send(t, &browserproto.LiveViewerMessageWatch{Tab: &tab})
	stream := v.until(
		t,
		"stream",
		func(raw json.RawMessage) bool { return string(observedField(t, raw, "width")) == "1600" },
	)
	expectValue(t, observedField(t, stream, "tab"), tab)
	expectValue(t, observedField(t, stream, "height"), 1200)
	generation, err := contract.Decode[uint32](observedField(t, stream, "generation"))
	if err != nil {
		t.Fatal(err)
	}
	frame := v.picture(t, generation)
	if !frame.header.Key || frame.header.Tab != tab || frame.header.Width != 1600 {
		t.Fatalf("first video %+v", frame.header)
	}
	if frame.header.Height != 1200 || !bytes.HasPrefix(frame.video, []byte{0, 0, 0, 1}) {
		t.Fatalf("video %+v", frame.header)
	}
}

// picture observes the first frame for the current stream, following replacement streams.
func (v *browserView) picture(t *testing.T, generation uint32) viewFrame {
	t.Helper()
	for {
		frame := v.next(t)
		if frame.control != nil && string(observedField(t, frame.control, "type")) == `"stream"` {
			next, err := contract.Decode[uint32](observedField(t, frame.control, "generation"))
			if err != nil {
				t.Fatal(err)
			}
			generation = next
		}
		if frame.video != nil && frame.header.Generation == generation {
			return frame
		}
	}
}

func (v *browserView) pointer(t *testing.T, tab browserproto.TabID, action browserproto.PointerAction, x, y float64) {
	buttons := uint8(0)
	if action == "down" {
		buttons = 1
	}
	v.send(
		t,
		&browserproto.LiveViewerMessagePointer{
			Tab:        tab,
			Action:     action,
			X:          x,
			Y:          y,
			Button:     "left",
			Buttons:    buttons,
			ClickCount: 1,
		},
	)
}

func (v *browserView) click(t *testing.T, tab browserproto.TabID, x, y float64) {
	v.pointer(t, tab, "down", x, y)
	v.pointer(t, tab, "up", x, y)
}

func (v *browserView) key(t *testing.T, tab browserproto.TabID, key, code string, keyCode uint8, text *string) {
	for _, action := range []browserproto.KeyAction{"down", "up"} {
		message := &browserproto.LiveViewerMessageKey{Tab: tab, Action: action, Key: key, Code: code, KeyCode: keyCode}
		if action == "down" {
			message.Text = text
		}
		v.send(t, message)
	}
}

func (v *browserView) close(t *testing.T) {
	t.Helper()
	v.closeInput.Do(func() { close(v.input.data) })
	for {
		select {
		case result := <-v.done:
			v.done <- result
			if result.err != nil || result.completion.ExitCode != 0 {
				t.Fatalf("live completion %+v", result)
			}
			return
		case <-v.records:
		case <-v.ctx.Done():
			t.Fatal(v.ctx.Err())
		}
	}
}

func (f *browserFixture) user(t *testing.T, name, args string) []byte {
	t.Helper()
	request := invocation(name, args, "acceptance")
	request.Request.Context.Caller = &commandproto.UserCaller{}
	request.Request.Cwd = f.root
	completion, stdout, stderr := call(t, f.s, request)
	if completion.ExitCode != 0 {
		t.Fatalf("user %s: %s", name, stderr)
	}
	return stdout
}

func TestAViewerWatchesATabAndTypesBesideTheAgent(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	view := f.view(t)
	view.hello(t, "linux")
	state := view.until(t, "state", nil)
	expectValue(t, observedField(t, state, "running"), true)
	expectValue(t, observedField(t, state, "tabs", "0", "id"), tab)
	expectValue(t, observedField(t, state, "tabs", "0", "createdBy", "kind"), "agent")
	expectValue(t, observedField(t, state, "watched"), nil)
	view.watch(t, tab)
	expectValue(
		t,
		observedField(t, f.command(t, tab, "info", "{}"), "viewport"),
		json.RawMessage(`{"width":800,"height":600,"devicePixelRatio":2,"mode":"web"}`),
	)
	expectValue(
		t,
		f.eval(
			t,
			tab,
			"[devicePixelRatio,screen.width,screen.height,innerWidth,outerWidth>=innerWidth,outerHeight>innerHeight]",
		),
		[]any{2, 1440, 900, 800, true, true},
	)
	for _, dy := range []float64{120, -120} {
		view.send(t, &browserproto.LiveViewerMessageWheel{Tab: tab, X: 400, Y: 300, DeltaY: dy})
		expression := "scrollY===120"
		if dy < 0 {
			expression = "scrollY===0"
		}
		f.eventually(t, tab, expression)
	}
	view.click(t, tab, 100, 25)
	view.key(t, tab, "h", "KeyH", 72, new("h"))
	view.key(t, tab, "i", "KeyI", 73, new("i"))
	f.eventually(
		t,
		tab,
		`document.querySelector('#text').value==='hi'&&events.includes('keypress:h')&&events.includes('input:text')`,
	)
	view.click(t, tab, 100, 90)
	view.key(t, tab, "a", "KeyA", 65, new("a"))
	view.key(t, tab, "Enter", "Enter", 13, nil)
	view.key(t, tab, "b", "KeyB", 66, new("b"))
	f.eventually(t, tab, `document.querySelector('#area').value==='a\nb'`)
	f.command(t, tab, "fill", `{"css":"#text","text":"agent"}`)
	view.click(t, tab, 100, 90)
	view.key(t, tab, "c", "KeyC", 67, new("c"))
	f.eventually(
		t,
		tab,
		`document.querySelector('#text').value==='agent'&&document.querySelector('#area').value==='a\nbc'`,
	)
	view.close(t)
}

func TestAViewWaitsForBrowserAndEndsWithItsLastTab(t *testing.T) {
	f := chromeFixture(t)
	view := f.view(t)
	view.hello(t, "linux")
	state := view.until(t, "state", nil)
	expectValue(t, observedField(t, state, "running"), false)
	expectValue(t, state, json.RawMessage(`{"type":"state","running":false,"tabs":[],"watched":null}`))
	opened, err := browserproto.DecodeOpenResult(f.user(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/live.html")))
	if err != nil {
		t.Fatal(err)
	}
	tab := opened.Tab
	state = view.until(
		t,
		"state",
		func(raw json.RawMessage) bool { return bytes.Contains(raw, mustBrowserValue(t, tab)) },
	)
	expectValue(t, observedField(t, state, "tabs", "0", "createdBy"), json.RawMessage(`{"kind":"user"}`))
	expectValue(t, observedField(t, state, "running"), true)
	expectValue(t, observedField(t, state, "watched"), nil)
	view.watch(t, tab)
	f.command(t, tab, "fill", `{"css":"#text","text":"retained across capture recovery"}`)
	second, err := browserproto.DecodeOpenResult(f.user(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/live.html")))
	if err != nil {
		t.Fatal(err)
	}
	state = view.until(
		t,
		"state",
		func(raw json.RawMessage) bool { return bytes.Contains(raw, mustBrowserValue(t, second.Tab)) },
	)
	rows, err := contract.List(observedField(t, state, "tabs"), contract.Decode[json.RawMessage])
	if err != nil || len(rows) != 2 {
		t.Fatalf("state %s: %v", state, err)
	}
	expectValue(t, observedField(t, state, "watched"), tab)
	view.watch(t, second.Tab)
	expectValue(t, f.read(t, tab, "#text", "value"), "retained across capture recovery")
	for _, id := range []browserproto.TabID{second.Tab, tab} {
		expectValue(
			t,
			f.user(t, "close", browserArgs(t, `{"tab":$0}`, id)),
			json.RawMessage(browserArgs(t, `{"closed":$0}`, id)),
		)
	}
	expectValue(t, observedField(t, view.until(t, "ended", nil), "reason"), "browser_ended")
	view.close(t)
	if len(f.tabs(t)) != 0 {
		t.Fatal("tabs survived")
	}
}

func TestTheUserCanImmediatelyCloseFirstLoadingTab(t *testing.T) {
	f := chromeFixture(t)
	closeLoading := func() {
		opened, err := browserproto.DecodeOpenResult(
			f.user(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/loading-form.html")),
		)
		if err != nil {
			t.Fatal(err)
		}
		expectValue(
			t,
			observedField(t, f.user(t, "close", browserArgs(t, `{"tab":$0,"timeout":3000}`, opened.Tab)), "closed"),
			opened.Tab,
		)
	}
	for range 3 {
		closeLoading()
	}
	keeper, err := browserproto.DecodeOpenResult(f.user(t, "open", `{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	for range 10 {
		closeLoading()
	}
	f.user(t, "close", browserArgs(t, `{"tab":$0}`, keeper.Tab))
}

func TestAViewerThatEndsReleasesOnlyWhatItHolds(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	view := f.view(t)
	view.hello(t, "linux")
	view.until(t, "state", nil)
	view.watch(t, tab)
	view.click(t, tab, 100, 25)
	view.send(
		t,
		&browserproto.LiveViewerMessageKey{
			Tab:       tab,
			Action:    "down",
			Key:       "Shift",
			Code:      "ShiftLeft",
			KeyCode:   16,
			Modifiers: 8,
			Location:  1,
		},
	)
	view.pointer(t, tab, "down", 400, 500)
	f.eventually(t, tab, `events.includes('keydown:Shift')&&events.filter(e=>e.startsWith('mousedown')).length===2`)
	view.close(t)
	f.eventually(t, tab, `events.includes('keyup:Shift')&&events.filter(e=>e.startsWith('mouseup')).length===2`)
	f.command(t, tab, "fill", `{"css":"#text","text":"after"}`)
	expectValue(t, f.read(t, tab, "#text", "value"), "after")
}

func TestModesFollowViewerAndAgent(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	view := f.view(t)
	view.hello(t, "mac")
	view.until(t, "state", nil)
	view.watch(t, tab)
	view.send(t, &browserproto.LiveViewerMessageMode{Tab: tab, Mode: "mobile"})
	stream := view.until(
		t,
		"stream",
		func(raw json.RawMessage) bool { return string(observedField(t, raw, "width")) == "780" },
	)
	expectValue(t, observedField(t, stream, "height"), 1688)
	for servedMobile := false; !servedMobile; {
		header := <-f.headers
		servedMobile = strings.Contains(header.Get("User-Agent"), "Android")
	}
	f.eventually(t, tab, `document.readyState==='complete'&&navigator.userAgent.includes('Android')`)
	expectValue(
		t,
		f.eval(
			t,
			tab,
			`[navigator.userAgentData.platform,navigator.userAgentData.mobile,navigator.userAgentData.brands.some(b=>b.brand==='Chromium'),innerWidth,navigator.maxTouchPoints,navigator.userAgent.includes('Android')]`,
		),
		[]any{"Android", true, true, 390, 5, true},
	)
	view.click(t, tab, 100, 25)
	f.eventually(t, tab, `events.includes('touchstart:text')&&events.includes('touchend:text')`)
	f.command(t, tab, "viewport.set", `{"width":1000,"height":700,"scale":1}`)
	state := view.until(
		t,
		"state",
		func(raw json.RawMessage) bool { return bytes.Contains(raw, []byte(`"mode":"custom"`)) },
	)
	expectValue(
		t,
		observedField(t, state, "tabs", "0", "viewport"),
		json.RawMessage(`{"width":1000,"height":700,"devicePixelRatio":1,"mode":"custom"}`),
	)
	expectValue(
		t,
		f.eval(t, tab, `[navigator.userAgent.includes('Android'),navigator.maxTouchPoints,innerWidth]`),
		[]any{false, 0, 1000},
	)
	view.send(t, &browserproto.LiveViewerMessageMode{Tab: tab, Mode: "web"})
	view.until(t, "stream", func(raw json.RawMessage) bool { return string(observedField(t, raw, "width")) == "1600" })
	expectValue(t, observedField(t, f.command(t, tab, "info", `{}`), "viewport", "mode"), "web")
	view.close(t)
}

func TestTwoViewersShareTabAndLastToOperateDecides(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	first := f.view(t)
	first.hello(t, "linux")
	first.until(t, "state", nil)
	first.watch(t, tab)
	second := f.view(t)
	second.send(t, &browserproto.LiveViewerMessageHello{Platform: "windows"})
	second.send(
		t,
		&browserproto.LiveViewerMessagePanel{
			Width:            1000,
			Height:           700,
			DevicePixelRatio: 1,
			ScreenWidth:      1920,
			ScreenHeight:     1080,
		},
	)
	second.until(t, "state", nil)
	second.send(t, &browserproto.LiveViewerMessageWatch{Tab: &tab})
	initial := second.until(t, "stream", nil)
	expectValue(t, observedField(t, initial, "width"), 1600)
	expectValue(t, observedField(t, initial, "height"), 1200)
	generation, err := contract.Decode[uint32](observedField(t, initial, "generation"))
	if err != nil {
		t.Fatal(err)
	}
	if frame := second.picture(t, generation); !frame.header.Key {
		t.Fatal("joining viewer did not receive a key frame")
	}
	second.click(t, tab, 100, 25)
	for _, view := range []*browserView{first, second} {
		stream := view.until(
			t,
			"stream",
			func(raw json.RawMessage) bool { return string(observedField(t, raw, "width")) == "1000" },
		)
		expectValue(t, observedField(t, stream, "height"), 700)
	}
	f.eventually(t, tab, `devicePixelRatio===1&&screen.width===1920&&innerWidth===1000&&innerHeight===700`)
	second.close(t)
	first.click(t, tab, 100, 25)
	first.until(t, "stream", func(raw json.RawMessage) bool { return string(observedField(t, raw, "width")) == "1600" })
	first.close(t)
}

func TestDialogsControlsFilesAndClipboardReachViewer(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "live.html")
	view := f.view(t)
	view.hello(t, "linux")
	view.until(t, "state", nil)
	view.send(t, &browserproto.LiveViewerMessageWatch{Tab: &tab})
	raw := view.until(t, "controls", func(raw json.RawMessage) bool {
		rows, err := contract.List(observedField(t, raw, "controls"), contract.Decode[json.RawMessage])
		return err == nil && len(rows) == 2
	})
	message, err := browserproto.DecodeLiveModuleMessage(raw)
	if err != nil {
		t.Fatal(err)
	}
	controls, ok := message.(*browserproto.LiveModuleMessageControls)
	if !ok {
		t.Fatalf("%T", message)
	}
	var choice, file *browserproto.LiveControl
	for i := range controls.Controls {
		control := &controls.Controls[i]
		if control.Kind == "select" {
			choice = control
		}
		if control.Kind == "file" {
			file = control
		}
	}
	if choice == nil || file == nil {
		t.Fatal(string(raw))
	}
	expectValue(t, observedField(t, mustBrowserValue(t, choice), "options", "1", "label"), "B")
	view.send(
		t,
		&browserproto.LiveViewerMessageChoice{
			Tab:      tab,
			Token:    choice.Token,
			Revision: choice.Revision,
			Value:    "b",
			Indices:  []uint32{1},
		},
	)
	expectValue(
		t,
		view.until(t, "choice", nil),
		json.RawMessage(browserArgs(t, `{"type":"choice","token":$0,"accepted":true}`, choice.Token)),
	)
	f.eventually(t, tab, `document.querySelector('#choice').value==='b'&&events.includes('change:choice')`)
	view.send(
		t,
		&browserproto.LiveViewerMessageChoice{
			Tab:      tab,
			Token:    choice.Token,
			Revision: 99,
			Value:    "a",
			Indices:  []uint32{0},
		},
	)
	expectValue(t, observedField(t, view.until(t, "choice", nil), "accepted"), false)
	view.send(
		t,
		&browserproto.LiveViewerMessageUpload{
			Tab:      tab,
			Token:    file.Token,
			Revision: file.Revision,
			Upload:   7,
			Files:    []browserproto.UploadFile{{Name: "notes.txt", MIMEType: "text/plain", Size: 11}},
		},
	)
	for _, data := range []string{"hello ", "world"} {
		frame := binary.BigEndian.AppendUint32(nil, uint32(9+len(data)))
		frame = append(frame, browserproto.FileFrame)
		frame = binary.BigEndian.AppendUint32(frame, 7)
		frame = binary.BigEndian.AppendUint32(frame, 0)
		frame = append(frame, data...)
		view.input.data <- frame
	}
	expectValue(t, observedField(t, view.until(t, "choice", nil), "accepted"), true)
	f.eventually(
		t,
		tab,
		`document.querySelector('#file').files[0]?.name==='notes.txt'&&document.querySelector('#file').files[0].size===11`,
	)
	view.click(t, tab, 50, 255)
	dialog := view.until(
		t,
		"dialog",
		func(raw json.RawMessage) bool { return string(observedField(t, raw, "dialog")) != "null" },
	)
	expectValue(
		t,
		observedField(t, dialog, "dialog"),
		json.RawMessage(`{"type":"alert","message":"hello","defaultText":""}`),
	)
	view.send(t, &browserproto.LiveViewerMessageDialog{Tab: tab, Accept: true})
	view.until(t, "dialog", func(raw json.RawMessage) bool { return string(observedField(t, raw, "dialog")) == "null" })
	view.click(t, tab, 100, 90)
	view.send(t, &browserproto.LiveViewerMessagePaste{Tab: tab, Text: "pasted", HTML: "<b>pasted</b>"})
	f.eventually(t, tab, `window.pasted?.text==='pasted'&&document.querySelector('#area').value==='pasted'`)
	view.click(t, tab, 50, 300)
	f.command(t, tab, "select-text", `{"css":"#copy","text":"copy me"}`)
	modifier := uint8(2)
	if runtime.GOOS == "darwin" {
		modifier = 4
	}
	for _, action := range []browserproto.KeyAction{"down", "up"} {
		view.send(
			t,
			&browserproto.LiveViewerMessageKey{
				Tab:       tab,
				Action:    action,
				Key:       "c",
				Code:      "KeyC",
				KeyCode:   67,
				Modifiers: modifier,
			},
		)
	}
	expectValue(t, observedField(t, view.until(t, "clipboard", nil), "text"), "copy me")
	view.send(t, &browserproto.LiveViewerMessageWheel{Tab: tab, X: 400, Y: 300, DeltaY: -200})
	f.eventually(t, tab, "scrollY===0")
	view.click(t, tab, 50, 355)
	expectValue(t, observedField(t, view.until(t, "clipboard", nil), "text"), "written")
	view.close(t)
}

func TestUsersRequestsAnswerWithoutWaitingForPage(t *testing.T) {
	f := chromeFixture(t)
	blank, err := browserproto.DecodeOpenResult(f.user(t, "open", `{"url":"about:blank"}`))
	if err != nil {
		t.Fatal(err)
	}
	tab := blank.Tab
	expectValue(t, mustBrowserValue(t, blank), json.RawMessage(browserArgs(t, `{"tab":$0,"url":"about:blank"}`, tab)))
	// /stall never answers, so an open that waited for its page could only fail.
	loading, err := browserproto.DecodeOpenResult(f.user(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/stall")))
	if err != nil {
		t.Fatal(err)
	}
	expectValue(
		t,
		mustBrowserValue(t, loading),
		json.RawMessage(browserArgs(t, `{"tab":$0,"url":$1}`, loading.Tab, f.url+"/stall")),
	)
	rows := f.tabs(t)
	if len(rows) != 2 || rows[0].ID != tab || rows[1].ID != loading.Tab {
		t.Fatal(rows)
	}
	for _, row := range rows {
		if _, ok := row.CreatedBy.(*browserproto.BrowserCreatedByUser); !ok {
			t.Fatal(row)
		}
	}
	request := invocation("back", browserArgs(t, `{"tab":$0}`, tab), "acceptance")
	request.Request.Context.Caller = &commandproto.UserCaller{}
	completion, _, stderr := call(t, f.s, request)
	failure, err := browserproto.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode != 1 || failure.Error.Code != "history_boundary" {
		t.Fatalf("%s %v", stderr, err)
	}
	request.Request.Operation = "browser.goto"
	request.Request.Args = []byte(browserArgs(t, `{"tab":$0,"url":"javascript:1"}`, tab))
	completion, _, stderr = call(t, f.s, request)
	failure, err = browserproto.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode != 2 || failure.Error.Code != "invalid_input" {
		t.Fatalf("%+v %s %v", completion, stderr, err)
	}
	base := f.url + "/live.html"
	for _, url := range []string{base, base + "?second"} {
		expectValue(
			t,
			f.user(t, "goto", browserArgs(t, `{"tab":$0,"url":$1}`, tab, url)),
			json.RawMessage(browserArgs(t, `{"tab":$0,"url":$1}`, tab, url)),
		)
		f.eventually(t, tab, `location.href===`+string(mustBrowserValue(t, url))+`&&document.readyState==='complete'`)
	}
	for _, step := range []struct{ name, url string }{{"back", base}, {"forward", base + "?second"}} {
		expectValue(
			t,
			f.user(t, step.name, browserArgs(t, `{"tab":$0}`, tab)),
			json.RawMessage(browserArgs(t, `{"tab":$0,"url":$1}`, tab, step.url)),
		)
		f.eventually(
			t,
			tab,
			`location.href===`+string(mustBrowserValue(t, step.url))+`&&document.readyState==='complete'`,
		)
	}
	request.Request.Operation = "browser.forward"
	request.Request.Args = []byte(browserArgs(t, `{"tab":$0}`, tab))
	completion, _, stderr = call(t, f.s, request)
	failure, err = browserproto.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode != 1 || failure.Error.Code != "history_boundary" {
		t.Fatalf("forward: %+v %s %v", completion, stderr, err)
	}
	origin := f.eval(t, tab, "performance.timeOrigin")
	expectValue(
		t,
		f.user(t, "reload", browserArgs(t, `{"tab":$0}`, tab)),
		json.RawMessage(browserArgs(t, `{"tab":$0,"url":$1}`, tab, base+"?second")),
	)
	f.eventually(t, tab, `performance.timeOrigin>`+string(origin)+`&&document.readyState==='complete'`)
	expectValue(
		t,
		f.user(t, "goto", browserArgs(t, `{"tab":$0,"url":$1}`, tab, f.url+"/stall")),
		json.RawMessage(browserArgs(t, `{"tab":$0,"url":$1}`, tab, f.url+"/stall")),
	)
	t.Run("missing tab", func(t *testing.T) {
		request := invocation("close", `{"tab":"t999"}`, "acceptance")
		request.Request.Context.Caller = &commandproto.UserCaller{}
		completion, _, stderr := call(t, f.s, request)
		failure, err := browserproto.DecodeFailureDocument(stderr)
		if err != nil || completion.ExitCode != 1 || failure.Error.Code != "tab_not_found" {
			t.Fatalf("close: %+v %s %v", completion, stderr, err)
		}
	})
	for _, id := range []browserproto.TabID{loading.Tab, tab} {
		expectValue(
			t,
			f.user(t, "close", browserArgs(t, `{"tab":$0}`, id)),
			json.RawMessage(browserArgs(t, `{"closed":$0}`, id)),
		)
	}
	if len(f.tabs(t)) != 0 {
		t.Fatal("user tabs survived")
	}
}
