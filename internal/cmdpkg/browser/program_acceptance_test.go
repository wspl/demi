//go:build acceptance && (darwin || linux)

package browser

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"image/png"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/programtest"
)

type browserProgram struct {
	process *cmdsdktest.ServiceProcess
	root    string
}

// startBrowserProgram serves runner requests and joins both streams with its child.
func startBrowserProgram(t *testing.T, home, temporary string) *browserProgram {
	t.Helper()
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to pinned Chrome")
	}
	path, err := programtest.Path(t.Context(), "demi-browser")
	if err != nil {
		t.Fatal(err)
	}
	process, err := cmdsdktest.Start(
		t.Context(),
		t,
		path,
		[]string{"--command-service"},
		[]string{"HOME=" + home, "TMPDIR=" + temporary},
	)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	numberStream, err := process.Client.Numbers(ctx)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	artifactStream, err := process.Client.Artifacts(ctx)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	done := make(chan error, 2)
	go func() {
		next := uint64(1)
		done <- numberStream.AnswerNumbers(ctx, func(_ context.Context, q commandwire.NumbersRequest) (uint64, error) {
			first := next
			next += uint64(q.Count)
			return first, nil
		})
	}()
	go func() {
		done <- artifactStream.AnswerArtifacts(ctx, func(_ context.Context, q commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
			if q.Installed != nil {
				version, err := tabs.PinnedVersion()
				if err != nil {
					return commandwire.ArtifactAnswer{}, err
				}
				installed := []commandwire.InstalledArtifact{{Version: version, SHA256: strings.Repeat("0", 64), Path: executable}}
				return commandwire.ArtifactAnswer{ID: q.ID, Installed: &installed}, nil
			}
			return commandwire.ArtifactAnswer{ID: q.ID, Path: &executable}, nil
		})
	}()
	t.Cleanup(func() {
		// A test may deliberately kill this child; Close still reaps it.
		_ = process.Close()
		cancel()
		<-done
		<-done
	})
	return &browserProgram{process: process, root: t.TempDir()}
}

// exchange collects an SDK command and rejects unexpected input pulls.
func exchangeBrowser(
	t *testing.T,
	client *cmdsdk.Client,
	request commandwire.Invocation,
) (commandwire.Completion, []byte, []byte) {
	t.Helper()
	input, output, err := client.Invoke(t.Context(), request)
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	return browserRecords(t, output)
}

// browserRecords observes command-service output and completion at its wire boundary.
func browserRecords(t *testing.T, output *cmdsdk.CommandOutput) (commandwire.Completion, []byte, []byte) {
	t.Helper()
	var stdout, stderr []byte
	var completion *commandwire.Completion
	for {
		record, err := output.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		switch r := record.(type) {
		case commandwire.Stdout:
			stdout = append(stdout, r...)
		case commandwire.Stderr:
			stderr = append(stderr, r...)
		case commandwire.Completed:
			completion = &r.Completion
		case commandwire.InputPull:
			t.Fatal("unexpected input pull")
		}
	}
	if completion == nil {
		t.Fatal("missing command completion")
	}
	return *completion, stdout, stderr
}

func (p *browserProgram) call(t *testing.T, name, args string) []byte {
	t.Helper()
	request := invocation(name, args, "service-program").Request
	request.Cwd = p.root
	completion, stdout, stderr := exchangeBrowser(t, p.process.Client, request)
	if completion.ExitCode != 0 {
		t.Fatalf("%s: %s", name, stderr)
	}
	return stdout
}

func (p *browserProgram) download(t *testing.T) string {
	t.Helper()
	path, err := filepath.Abs("testdata/download.html")
	if err != nil {
		t.Fatal(err)
	}
	tab := observedField(t, p.call(t, "open", browserArgs(t, `{"url":$0}`, "file://"+path)), "tab")
	result := p.call(t, "download", browserArgs(t, `{"tab":$0,"css":"#instant"}`, tab))
	saved, err := contract.Decode[string](observedField(t, result, "path"))
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(saved)
	if err != nil || string(data) != "download fixture\n" {
		t.Fatalf("%s %v", data, err)
	}
	return saved
}

func TestChromeKeepsDirectoriesApartWhateverServiceHomeAndTemporaryDirectory(t *testing.T) {
	scratch, home := t.TempDir(), t.TempDir()
	temporary := filepath.Join(scratch, strings.Repeat("t", max(1, 90-len(scratch)-1)))
	if err := os.Mkdir(temporary, 0o700); err != nil {
		t.Fatal(err)
	}
	service := startBrowserProgram(t, home, temporary)
	saved := service.download(t)
	profile := filepath.Dir(filepath.Dir(saved))
	if filepath.Dir(profile) != temporary {
		t.Fatal(profile)
	}
	socket, err := os.Readlink(filepath.Join(profile, "SingletonSocket"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(socket, "/tmp/") {
		t.Fatal(socket)
	}
	if err := service.process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
	entries, err := os.ReadDir(temporary)
	if err != nil || len(entries) != 0 {
		t.Fatalf("retained %v %v", entries, err)
	}
	allowed := ".local/share/pki"
	if runtime.GOOS == "darwin" {
		allowed = "Library/Application Support/Google/Chrome for Testing"
	}
	err = filepath.WalkDir(home, func(path string, _ fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(home, path)
		if err != nil {
			return err
		}
		if relative != "." && !strings.HasPrefix(relative, allowed) && !strings.HasPrefix(allowed, relative) {
			t.Errorf("Chrome wrote in home: %s", relative)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestWithoutTMPDIRProfilesAndDownloadsGoToVarTmp(t *testing.T) {
	service := startBrowserProgram(t, t.TempDir(), "")
	saved := service.download(t)
	profile := filepath.Dir(filepath.Dir(saved))
	if filepath.Dir(profile) != "/var/tmp" {
		t.Fatal(profile)
	}
	if err := service.process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{profile, saved} {
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Fatalf("retained %s: %v", path, err)
		}
	}
}

func TestNextServiceSweepsKilledServicesBrowserAndDownloads(t *testing.T) {
	home, temporary := t.TempDir(), t.TempDir()
	killed := startBrowserProgram(t, home, temporary)
	saved := killed.download(t)
	profile := filepath.Dir(filepath.Dir(saved))
	leader := profileOwner(t, profile)
	runtimeDirectory := profileRuntime(t, profile)
	owned := markedBrowserProcesses(t, runtimeDirectory)
	t.Cleanup(func() {
		for _, pid := range owned {
			if err := syscall.Kill(pid, syscall.SIGKILL); err != nil && !errors.Is(err, syscall.ESRCH) {
				t.Error(err)
			}
		}
		// The killed processes are not this test's children, so no wait call
		// reports their end; each attempt is a real process listing.
		for len(markedBrowserProcesses(t, runtimeDirectory)) != 0 {
			runtime.Gosched()
		}
		if err := os.RemoveAll(runtimeDirectory); err != nil {
			t.Error(err)
		}
	})
	if err := killed.process.Close(); err == nil {
		t.Fatal("killed service succeeded")
	}
	next := startBrowserProgram(t, home, temporary)
	if err := next.process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
	entries, err := os.ReadDir(temporary)
	if err != nil || len(entries) != 0 {
		t.Errorf("sweep retained %v %v", entries, err)
	}
	if group := processGroup(t, leader); len(group) > 0 {
		// Preserve ownership even when the sweep regression fails.
		if err := syscall.Kill(-leader, syscall.SIGKILL); err != nil && !errors.Is(err, syscall.ESRCH) {
			t.Error(err)
		}
		t.Errorf("sweep retained process group %v", group)
	}
}

func TestResidentConversationBrowserCommandsShareStateAndRetire(t *testing.T) {
	p := startBrowserProgram(t, t.TempDir(), t.TempDir())
	path, err := filepath.Abs("testdata/fixture.html")
	if err != nil {
		t.Fatal(err)
	}
	expectValue(t, observedField(t, p.call(t, "tabs", `{}`), "tabs"), []string{})
	tab := observedField(t, p.call(t, "open", browserArgs(t, `{"url":$0}`, "file://"+path)), "tab")
	tree := observedField(t, p.call(t, "inspect", browserArgs(t, `{"tab":$0,"limit":1000}`, tab)), "tree")
	nodes, err := contract.List(tree, contract.Decode[json.RawMessage])
	if err != nil || len(nodes) == 0 {
		t.Fatalf("inspect %s: %v", tree, err)
	}
	p.call(t, "goto", browserArgs(t, `{"tab":$0,"url":$1}`, tab, "file://"+path+"?navigated"))
	p.call(t, "fill", browserArgs(t, `{"tab":$0,"css":"#email","text":"浏览器@example.test"}`, tab))
	p.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#normal"}`, tab))
	expectValue(
		t,
		observedField(
			t,
			p.call(t, "read", browserArgs(t, `{"tab":$0,"css":"#email","property":"value"}`, tab)),
			"value",
		),
		"浏览器@example.test",
	)
	for _, test := range []struct{ name, args string }{{"screenshot", `{"tab":$0,"output":"browser.png"}`}, {"screenshot", `{"tab":$0,"output":"clip.png","clip":"0,0,100,80"}`}, {"probe", `{"tab":$0,"xy":"30,30","include-non-interactable":true,"output":"probe.png"}`}} {
		result := p.call(t, test.name, browserArgs(t, test.args, tab))
		if test.name == "screenshot" {
			expectValue(t, observedField(t, result, "mimeType"), "image/png")
		} else {
			matches, err := contract.List(observedField(t, result, "matches"), contract.Decode[json.RawMessage])
			if err != nil || len(matches) == 0 {
				t.Fatalf("probe %s: %v", result, err)
			}
		}
		path, err := contract.Decode[string](observedField(t, result, "path"))
		if err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		config, err := png.DecodeConfig(bytes.NewReader(data))
		if err != nil {
			t.Fatal(err)
		}
		if strings.HasSuffix(path, "clip.png") && (config.Width != 100 || config.Height != 80) {
			t.Fatal(config)
		}
	}
	expectValue(t, observedField(t, p.call(t, "tabs", `{"offset":1,"limit":1}`), "tabs"), []string{})
	for _, test := range []struct {
		name, args, code string
		exit             uint8
	}{{"screenshot", `{"tab":$0,"output":"browser.png"}`, "output_exists", 1}, {"screenshot", `{"tab":$0,"output":"missing-directory/browser.png"}`, "io_error", 1}, {"info", `{"tab":"t999"}`, "tab_not_found", 1}, {"key", `{"tab":$0,"css":"#email","key":"not-a-key"}`, "invalid_input", 2}} {
		request := invocation(test.name, browserArgs(t, test.args, tab), "service-program").Request
		request.Cwd = p.root
		completion, _, stderr := exchangeBrowser(t, p.process.Client, request)
		failure, err := browserop.DecodeFailureDocument(stderr)
		if err != nil || completion.ExitCode != test.exit || string(failure.Error.Code) != test.code {
			t.Fatalf("%+v %s %v", completion, stderr, err)
		}
		if test.name == "key" &&
			(failure.Error.Details == nil || failure.Error.Details.Action == nil || *failure.Error.Details.Action != "not_started") {
			t.Fatalf("invalid key details: %s", stderr)
		}
	}
	for _, name := range []string{"key", "inspect"} {
		args := browserArgs(t, `{"tab":$0,"limit":1000}`, tab)
		if name == "key" {
			args = browserArgs(t, `{"tab":$0,"css":"#email","key":"not-a-key"}`, tab)
		}
		request := invocation(name, args, "service-program").Request
		request.JSON = new(false)
		completion, stdout, stderr := exchangeBrowser(t, p.process.Client, request)
		if name == "key" {
			if completion.ExitCode != 2 || !bytes.HasPrefix(stderr, []byte("Error: invalid_input\n")) ||
				!bytes.Contains(stderr, []byte("Tab: "+strings.Trim(string(tab), "\"")+"\n")) ||
				!bytes.Contains(stderr, []byte("Action: not_started.")) ||
				bytes.Contains(stderr, []byte("Details: {")) {
				t.Fatalf("%+v %s", completion, stderr)
			}
		} else if completion.ExitCode != 0 || !bytes.Contains(stdout, []byte("[checked=false]")) || !bytes.Contains(stdout, []byte(`[value="浏览器@example.test"]`)) || !bytes.Contains(stdout, []byte("[protected]")) || bytes.Contains(stdout, []byte("fixture-secret")) {
			t.Fatal(string(stdout))
		}
	}
	p.call(t, "click", browserArgs(t, `{"tab":$0,"css":"#popup"}`, tab))
	p.call(t, "close", browserArgs(t, `{"tab":$0}`, tab))
	listed, err := browserop.DecodeTabsResult(p.call(t, "tabs", `{}`))
	if err != nil || len(listed.Tabs) != 1 {
		t.Fatalf("%+v %v", listed, err)
	}
	expectValue(t, observedField(t, mustBrowserValue(t, listed.Tabs[0]), "createdBy", "opener"), tab)
	p.call(t, "close", browserArgs(t, `{"tab":$0}`, listed.Tabs[0].ID))
	input, output, err := p.process.Client.Conversation(t.Context(), &commandwire.ConversationQuery{})
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	completion, stdout, stderr := browserRecords(t, output)
	if completion.ExitCode != 0 {
		t.Fatalf("query %+v %s", completion, stderr)
	}
	expectValue(t, stdout, json.RawMessage(`{"conversations":[]}`))
	p.call(t, "open", `{"url":"about:blank"}`)
	input, output, err = p.process.Client.Conversation(
		t.Context(),
		&commandwire.ConversationRelease{Conversation: "service-program"},
	)
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	completion, stdout, stderr = browserRecords(t, output)
	if completion.ExitCode != 0 {
		t.Fatalf("release %+v %s", completion, stderr)
	}
	expectValue(t, stdout, json.RawMessage("{}"))
	if err := p.process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
}
