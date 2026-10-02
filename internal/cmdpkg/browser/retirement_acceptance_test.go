//go:build acceptance && (darwin || linux)

package browser

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandwire"
)

// profileOwner reads Chrome's own singleton record for this fixture's profile.
func profileOwner(t *testing.T, profile string) int {
	t.Helper()
	link, err := os.Readlink(filepath.Join(profile, "SingletonLock"))
	if err != nil {
		t.Fatal(err)
	}
	_, pid, ok := strings.Cut(link, "-")
	if at := strings.LastIndexByte(link, '-'); at >= 0 {
		pid = link[at+1:]
		ok = true
	}
	number, err := strconv.Atoi(pid)
	if !ok || err != nil || number <= 0 {
		t.Fatalf("profile owner %q: %v", link, err)
	}
	return number
}

// markedBrowserProcesses identifies only this fixture's Chrome and detached helpers.
func markedBrowserProcesses(t *testing.T, directory string) []int {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.WithoutCancel(t.Context()), 5*time.Second)
	defer cancel()
	data, err := exec.CommandContext(ctx, "ps", "eww", "-axo", "pid=,command=").Output()
	if err != nil {
		t.Fatal(err)
	}
	found := []int{}
	for _, line := range strings.Split(string(data), "\n") {
		fields := strings.Fields(line)
		for _, field := range fields {
			if field != "DEMI_BROWSER_PROFILE="+directory {
				continue
			}
			pid, err := strconv.Atoi(fields[0])
			if err != nil {
				t.Fatal(err)
			}
			found = append(found, pid)
			break
		}
	}
	return found
}

// processGroup reads independent OS evidence of Chrome's process-tree retirement.
func processGroup(t *testing.T, group int) []int {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.WithoutCancel(t.Context()), 5*time.Second)
	defer cancel()
	data, err := exec.CommandContext(ctx, "ps", "-axo", "pid=,pgid=").Output()
	if err != nil {
		t.Fatal(err)
	}
	found := []int{}
	for _, line := range strings.Split(string(data), "\n") {
		fields := strings.Fields(line)
		if len(fields) == 0 {
			continue
		}
		if len(fields) != 2 {
			t.Fatalf("invalid process row %q", line)
		}
		pid, err := strconv.Atoi(fields[0])
		if err != nil {
			t.Fatal(err)
		}
		pgid, err := strconv.Atoi(fields[1])
		if err != nil {
			t.Fatal(err)
		}
		if pgid == group {
			found = append(found, pid)
		}
	}
	return found
}

func TestConversationReleaseCancelsOnlyItsCommandsAndRetiresProfile(t *testing.T) {
	first := chromeFixture(t)
	second := *first
	second.conversation = "second-conversation"
	firstTab := first.open(t, "fixture.html")
	profile := filepath.Dir(first.environment(t).DownloadDirectory())
	leader := profileOwner(t, profile)
	if len(processGroup(t, leader)) < 2 {
		t.Fatal("Chrome has no helpers")
	}
	secondTab := second.open(t, "fixture.html")
	otherProfile := filepath.Dir(second.environment(t).DownloadDirectory())
	expectValue(t, callLifecycle(t, first.s, &commandwire.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{"acceptance", "second-conversation"}})
	waiting := first.start(t, "wait", browserArgs(t, `{"tab":$0,"url":"**/never","timeout":30000}`, firstTab))
	first.waitBusy(t, firstTab)
	callLifecycle(t, first.s, &commandwire.ConversationRelease{Conversation: first.conversation})
	requireCancelled(t, waiting.join(t))
	if _, err := os.Stat(profile); !os.IsNotExist(err) {
		t.Fatalf("profile survived: %v", err)
	}
	if group := processGroup(t, leader); len(group) != 0 {
		t.Fatal(group)
	}
	if _, err := os.Stat(otherProfile); err != nil {
		t.Fatal(err)
	}
	rows := second.tabs(t)
	if len(rows) != 1 || rows[0].ID != secondTab {
		t.Fatal(rows)
	}
	if len(first.tabs(t)) != 0 {
		t.Fatal("released tabs survived")
	}
	if fresh := first.open(t, "fixture.html"); fresh == firstTab {
		t.Fatal("reused tab")
	}
	for _, id := range []string{first.conversation, second.conversation} {
		callLifecycle(t, first.s, &commandwire.ConversationRelease{Conversation: id})
	}
	expectValue(t, callLifecycle(t, first.s, &commandwire.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{}})
	if _, err := os.Stat(otherProfile); !os.IsNotExist(err) {
		t.Fatal(err)
	}
}

func TestBrowserUsesTrustedConversationAndCallerDespiteScriptEnvironment(t *testing.T) {
	first := chromeFixture(t)
	first.caller = 1
	second := *first
	second.conversation = "other-conversation"
	second.caller = 2
	second.open(t, "fixture.html")
	secondTab := second.open(t, "fixture.html")
	first.env = map[string]string{"DEMI_CONVERSATION_ID": second.conversation, "DEMI_AGENT_NODE_ID": "2"}
	firstTab := first.open(t, "fixture.html")
	rows := first.tabs(t)
	if len(rows) != 1 || rows[0].ID != firstTab {
		t.Fatal(rows)
	}
	creator, ok := rows[0].CreatedBy.(*browserop.BrowserCreatedByAgent)
	if !ok || creator.Number != 1 {
		t.Fatal(rows)
	}
	first.rejects(t, secondTab, "info", `{}`, "tab_not_found", "")
	if rows := second.tabs(t); len(rows) != 2 || rows[1].ID != secondTab {
		t.Fatal(rows)
	}
	first.command(t, firstTab, "close", `{}`)
	expectValue(t, callLifecycle(t, first.s, &commandwire.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{second.conversation}})
	if fresh := first.open(t, "fixture.html"); fresh == firstTab {
		t.Fatal("reused tab")
	}
	first.rejects(t, firstTab, "info", `{}`, "tab_not_found", "")
}

func TestLastTabCloseFailsRunningCommandAsBrowserLost(t *testing.T) {
	f := chromeFixture(t)
	tab := f.open(t, "fixture.html")
	job := f.start(t, "wait", browserArgs(t, `{"tab":$0,"url":"**/never","timeout":30000}`, tab))
	f.waitBusy(t, tab)
	f.command(t, tab, "close", `{}`)
	result := job.join(t)
	failure, err := browserop.DecodeFailureDocument(result.stderr)
	if err != nil || result.completion.ExitCode != 1 || failure.Error.Code != "browser_lost" {
		t.Fatalf("%+v %s %v", result, result.stderr, err)
	}
	if failure.Error.Details == nil || failure.Error.Details.Action == nil || *failure.Error.Details.Action != "not_started" {
		t.Fatal(failure)
	}
	expectValue(t, callLifecycle(t, f.s, &commandwire.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{}})
}

func TestNewOpenRecoversAfterChromeCrashWithoutReplayingOldTabs(t *testing.T) {
	f := chromeFixture(t)
	old := f.open(t, "cdp.html")
	for _, method := range []string{"Debugger.enable", "Fetch.enable"} {
		f.command(t, old, "cdp.send", browserArgs(t, `{"method":$0,"params":"{}"}`, method))
	}
	profile := filepath.Dir(f.environment(t).DownloadDirectory())
	leader := profileOwner(t, profile)
	if err := syscall.Kill(leader, syscall.SIGKILL); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	for {
		if err := ctx.Err(); err != nil {
			t.Fatal(err)
		}
		if _, err := os.Stat(profile); os.IsNotExist(err) {
			break
		} else if err != nil {
			t.Fatal(err)
		}
		runtime.Gosched()
	}
	fresh := f.open(t, "cdp.html")
	if fresh == old {
		t.Fatal("old tab replayed")
	}
	rows := f.tabs(t)
	if len(rows) != 1 || rows[0].ID != fresh {
		t.Fatal(rows)
	}
	f.rejects(t, old, "info", `{}`, "tab_not_found", "")
}

// Four launches cost roughly eight seconds locally; each exit path must join Chrome.
func TestChromeProcessTreeAndProfileRetireTogether(t *testing.T) {
	executable := os.Getenv("DEMI_TEST_CHROME")
	if executable == "" {
		t.Skip("set DEMI_TEST_CHROME to pinned Chrome")
	}
	for _, mode := range []string{"success", "failure", "cancel", "killed"} {
		t.Run(mode, func(t *testing.T) {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			env := tabstest.Launch(ctx, t, tabs.LaunchOptions{Executable: executable, Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}})
			if _, err := env.Open(ctx, "about:blank", 5*time.Second); err != nil {
				t.Fatal(err)
			}
			profile := filepath.Dir(env.DownloadDirectory())
			leader := profileOwner(t, profile)
			socket, err := os.Readlink(filepath.Join(profile, "SingletonSocket"))
			if err != nil {
				t.Fatal(err)
			}
			runtimeDirectory := filepath.Dir(filepath.Dir(socket))
			if filepath.Dir(runtimeDirectory) != tabs.HostDirectories().Runtime {
				t.Fatal(runtimeDirectory)
			}
			for _, path := range []string{profile, runtimeDirectory} {
				info, err := os.Stat(path)
				if err != nil || info.Mode().Perm() != 0700 {
					t.Fatalf("%s %v %v", path, info, err)
				}
			}
			if group := processGroup(t, leader); len(group) < 2 {
				t.Fatal(group)
			}
			injected := errors.New("injected work failure")
			result := func() (err error) {
				defer func() { err = errors.Join(err, env.Close(context.Background())) }()
				switch mode {
				case "failure":
					return injected
				case "cancel":
					cancel()
					<-env.Done()
					return ctx.Err()
				case "killed":
					if err := syscall.Kill(leader, syscall.SIGKILL); err != nil {
						return err
					}
					<-env.Done()
					return env.Failure()
				default:
					return nil
				}
			}()
			switch mode {
			case "success":
				if result != nil {
					t.Fatal(result)
				}
			case "failure":
				if !errors.Is(result, injected) {
					t.Fatal(result)
				}
			case "cancel":
				if !errors.Is(result, context.Canceled) {
					t.Fatal(result)
				}
			case "killed":
				if result == nil {
					t.Fatal("crash succeeded")
				}
			}
			if group := processGroup(t, leader); len(group) != 0 {
				t.Fatal(group)
			}
			for _, path := range []string{profile, runtimeDirectory} {
				if _, err := os.Stat(path); !os.IsNotExist(err) {
					t.Fatalf("retained %s: %v", path, err)
				}
			}
		})
	}
}

// A child test deliberately fails; its cleanup must finish before that failure exits.
func TestFixtureAssertionsRetireChromeAndProfilesBeforeFailureReturns(t *testing.T) {
	if os.Getenv("DEMI_BROWSER_TEST_ASSERTION") == "1" {
		for _, harness := range []string{"direct", "service"} {
			t.Run(harness, func(t *testing.T) {
				var profile string
				var leader int
				t.Cleanup(func() {
					if profile == "" {
						return
					}
					if _, err := os.Stat(profile); !os.IsNotExist(err) {
						t.Errorf("profile survived: %v", err)
						return
					}
					if len(processGroup(t, leader)) != 0 {
						t.Error("processes survived")
						return
					}
					fmt.Println("retired after deliberate assertion: " + harness)
				})
				if harness == "service" {
					f := chromeFixture(t)
					f.open(t, "repairs.html")
					profile = filepath.Dir(f.environment(t).DownloadDirectory())
				} else {
					env := tabstest.Launch(t.Context(), t, tabs.LaunchOptions{Executable: os.Getenv("DEMI_TEST_CHROME"), Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}})
					if _, err := env.Open(t.Context(), "about:blank", 5*time.Second); err != nil {
						t.Fatal(err)
					}
					profile = filepath.Dir(env.DownloadDirectory())
				}
				leader = profileOwner(t, profile)
				t.Fatal("deliberate fixture assertion")
			})
		}
		return
	}
	if os.Getenv("DEMI_TEST_CHROME") == "" {
		t.Skip("set DEMI_TEST_CHROME to pinned Chrome")
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	child := exec.CommandContext(t.Context(), executable, "-test.run=^TestFixtureAssertionsRetireChromeAndProfilesBeforeFailureReturns$", "-test.count=1", "-test.v")
	child.Env = append(os.Environ(), "DEMI_BROWSER_TEST_ASSERTION=1")
	output, err := child.CombinedOutput()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 1 {
		t.Fatalf("child %s: %v", output, err)
	}
	for _, harness := range []string{"direct", "service"} {
		if !strings.Contains(string(output), "retired after deliberate assertion: "+harness) {
			t.Fatal(string(output))
		}
	}
}
