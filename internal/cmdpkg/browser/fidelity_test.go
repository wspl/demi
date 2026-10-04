//go:build acceptance

package browser

import (
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/commandwire"
)

// eventually evaluates the condition in the page until it holds. Each attempt is a
// real evaluation, so it never sleeps; go test -timeout guards against a hang.
func (f *browserFixture) eventually(t *testing.T, tab browserop.TabID, expression string) {
	t.Helper()
	for {
		completion, stdout, _ := f.result(t, "eval", browserArgs(t, `{"tab":$0,"expression":$1}`, tab, expression))
		if completion.ExitCode == 0 && string(observedField(t, stdout, "value")) == "true" {
			return
		}
	}
}

func TestPagesSeeOrdinaryChromeInUsersTimeZoneAndLanguages(t *testing.T) {
	f := chromeFixture(t)
	f.locale = &commandwire.CommandLocale{
		TimeZone:  "America/Sao_Paulo",
		Languages: []commandwire.LanguageTag{"pt-BR", "en"},
	}
	tab := f.open(t, "fidelity.html")
	f.eventually(t, tab, "!!window.report")
	report := f.eval(t, tab, "window.report")
	expectValue(t, observedField(t, report, "webdriver"), false)
	if string(observedField(t, report, "worker", "webdriver")) == "true" {
		t.Fatal(string(report))
	}
	agent := observedField(t, report, "userAgent")
	if strings.Contains(string(agent), "Headless") || !strings.Contains(string(agent), ".0.0.0 Safari/537.36") {
		t.Fatal(string(agent))
	}
	expectValue(t, observedField(t, report, "worker", "userAgent"), agent)
	expectValue(
		t,
		f.eval(
			t,
			tab,
			`report.brands.includes('Chromium')&&report.outer[0]>=report.inner[0]&&report.outer[1]>report.inner[1]&&report.hover&&report.finePointer`,
		),
		true,
	)
	if runtime.GOOS != "darwin" {
		expectValue(t, f.eval(t, tab, "report.scrollbar>0"), true)
	}
	for _, path := range [][]string{{"timeZone"}, {"worker", "timeZone"}} {
		expectValue(t, observedField(t, report, path...), "America/Sao_Paulo")
	}
	for _, path := range [][]string{{"language"}, {"worker", "language"}, {"defaultLocale"}} {
		expectValue(t, observedField(t, report, path...), "pt-BR")
	}
	header := <-f.headers
	if !strings.HasPrefix(header.Get("Accept-Language"), "pt-BR") ||
		strings.Contains(header.Get("User-Agent"), "Headless") {
		t.Fatal(header)
	}
	f.locale = &commandwire.CommandLocale{TimeZone: "Asia/Tokyo", Languages: []commandwire.LanguageTag{"ja"}}
	second := f.open(t, "fidelity.html")
	f.eventually(t, second, "!!window.report")
	expectValue(t, f.eval(t, second, "report.timeZone"), "America/Sao_Paulo")
}

func TestAgentViewportSetsPixelRatioAndScreenshotsStayInCSSPixels(t *testing.T) {
	f := chromeFixture(t)
	opened := f.call(t, "open", browserArgs(t, `{"url":$0}`, f.url+"/fidelity.html"))
	result, err := browserop.DecodeOpenResult(opened)
	if err != nil {
		t.Fatal(err)
	}
	tab := result.Tab
	web := browserop.BrowserViewport{Width: 1280, Height: 720, DevicePixelRatio: 1, Mode: "web"}
	expectValue(t, observedField(t, opened, "viewport"), web)
	expectValue(t, f.eval(t, tab, "[innerWidth,innerHeight,devicePixelRatio]"), []int{1280, 720, 1})
	set := observedField(t, f.command(t, tab, "viewport.set", `{"width":800,"height":600,"scale":2}`), "viewport")
	expectValue(t, set, browserop.BrowserViewport{Width: 800, Height: 600, DevicePixelRatio: 2, Mode: "custom"})
	expectValue(t, observedField(t, f.command(t, tab, "info", `{}`), "viewport"), set)
	expectValue(
		t,
		f.eval(t, tab, "[devicePixelRatio,innerWidth,innerHeight,outerHeight>innerHeight]"),
		[]any{2, 800, 600, true},
	)
	f.command(t, tab, "scroll", `{"xy":"400,300","dy":500}`)
	for _, test := range []struct {
		args          string
		width, height int
	}{{`{"output":"shot.png"}`, 800, 600}, {`{"clip":"0,0,100,50","output":"clip.png"}`, 100, 50}} {
		shot := f.command(t, tab, "screenshot", test.args)
		expectValue(t, observedField(t, shot, "width"), test.width)
		expectValue(t, observedField(t, shot, "height"), test.height)
		expectValue(t, observedField(t, shot, "viewport"), set)
	}
	expectValue(t, observedField(t, f.command(t, tab, "viewport.reset", `{}`), "viewport"), web)
	shot := f.command(t, tab, "screenshot", `{"output":"back.png"}`)
	expectValue(t, observedField(t, shot, "width"), 1280)
	expectValue(t, observedField(t, shot, "height"), 720)
	completion, _, stderr := f.result(
		t,
		"viewport.set",
		browserArgs(t, `{"tab":$0,"width":800,"height":600,"scale":8}`, tab),
	)
	if completion.ExitCode != 2 {
		t.Fatalf("%+v %s", completion, stderr)
	}
}
