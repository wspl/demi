//go:build acceptance && (darwin || linux)

package browser

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandproto"
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

// profileRuntime reads Chrome's socket location to identify its private runtime directory.
func profileRuntime(t *testing.T, profile string) string {
	t.Helper()
	socket, err := os.Readlink(filepath.Join(profile, "SingletonSocket"))
	if err != nil {
		t.Fatal(err)
	}
	return filepath.Dir(filepath.Dir(socket))
}

// markedBrowserProcesses identifies only this fixture's Chrome and detached helpers.
func markedBrowserProcesses(t *testing.T, directory string) []int {
	t.Helper()
	// Cleanup calls this after the test's context is canceled.
	data, err := exec.CommandContext(context.WithoutCancel(t.Context()), "ps", "eww", "-axo", "pid=,command=").Output()
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
	// Cleanup calls this after the test's context is canceled.
	data, err := exec.CommandContext(context.WithoutCancel(t.Context()), "ps", "-axo", "pid=,pgid=").Output()
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

// chromeProfiles observes live Chrome profiles belonging to this test process.
func chromeProfiles(t *testing.T) map[int]string {
	t.Helper()
	data, err := exec.CommandContext(t.Context(), "ps", "-axo", "pid=,ppid=").Output()
	if err != nil {
		t.Fatal(err)
	}
	children := make(map[int]bool)
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
		parent, err := strconv.Atoi(fields[1])
		if err != nil {
			t.Fatal(err)
		}
		if parent == os.Getpid() {
			children[pid] = true
		}
	}
	base := tabs.HostDirectories().Profiles
	entries, err := os.ReadDir(base)
	if err != nil {
		t.Fatal(err)
	}
	profiles := make(map[int]string)
	for _, entry := range entries {
		if !strings.HasPrefix(entry.Name(), "demi-profile-") {
			continue
		}
		path := filepath.Join(base, entry.Name())
		lock, err := os.Readlink(filepath.Join(path, "SingletonLock"))
		if errors.Is(err, os.ErrNotExist) || errors.Is(err, os.ErrPermission) {
			continue
		}
		if err != nil {
			t.Fatal(err)
		}
		at := strings.LastIndexByte(lock, '-')
		if at < 0 {
			t.Fatalf("invalid profile owner %q", lock)
		}
		owner, err := strconv.Atoi(lock[at+1:])
		if err != nil {
			t.Fatal(err)
		}
		if children[owner] {
			profiles[owner] = path
		}
	}
	return profiles
}

func TestConversationReleaseCancelsOnlyItsCommandsAndRetiresProfile(t *testing.T) {
	first := chromeFixture(t)
	second := *first
	second.conversation = "second-conversation"
	firstTab := first.open(t, "fixture.html")
	profile := filepath.Dir(first.environment(t).DownloadDirectory())
	leader := profileOwner(t, profile)
	if profiles := chromeProfiles(t); len(profiles) != 1 || profiles[leader] != profile {
		t.Fatal(profiles)
	}
	descendants := processGroup(t, leader)
	if len(descendants) < 2 {
		t.Fatal("Chrome has no helpers")
	}
	secondTab := second.open(t, "fixture.html")
	otherProfile := filepath.Dir(second.environment(t).DownloadDirectory())
	profiles := chromeProfiles(t)
	if len(profiles) != 2 {
		t.Fatal(profiles)
	}
	delete(profiles, leader)
	expectValue(t, callLifecycle(t, first.s, &commandproto.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{"acceptance", "second-conversation"}})
	waiting := first.start(t, "wait", browserArgs(t, `{"tab":$0,"url":"**/never","timeout":30000}`, firstTab))
	first.waitBusy(t, firstTab)
	expectValue(
		t,
		callLifecycle(t, first.s, &commandproto.ConversationRelease{Conversation: first.conversation}),
		json.RawMessage("{}"),
	)
	expectValue(
		t,
		callLifecycle(t, first.s, &commandproto.ConversationQuery{}),
		json.RawMessage(`{"conversations":["second-conversation"]}`),
	)
	requireCancelled(t, waiting.join(t))
	if _, err := os.Stat(profile); !os.IsNotExist(err) {
		t.Fatalf("profile survived: %v", err)
	}
	if group := processGroup(t, leader); len(group) != 0 {
		t.Fatal(group)
	}
	for _, pid := range descendants {
		if err := syscall.Kill(pid, 0); !errors.Is(err, syscall.ESRCH) {
			t.Fatalf("Chrome helper %d survived: %v", pid, err)
		}
	}
	if after := chromeProfiles(t); !maps.Equal(after, profiles) {
		t.Fatal(after)
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
	freshProfile := filepath.Dir(first.environment(t).DownloadDirectory())
	for _, id := range []string{first.conversation, second.conversation} {
		expectValue(
			t,
			callLifecycle(t, first.s, &commandproto.ConversationRelease{Conversation: id}),
			json.RawMessage("{}"),
		)
	}
	expectValue(t, callLifecycle(t, first.s, &commandproto.ConversationQuery{}), struct {
		Conversations []string `json:"conversations"`
	}{[]string{}})
	if profiles := chromeProfiles(t); len(profiles) != 0 {
		t.Fatal(profiles)
	}
	for _, profile := range []string{otherProfile, freshProfile} {
		if _, err := os.Stat(profile); !os.IsNotExist(err) {
			t.Fatal(err)
		}
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
	creator, ok := rows[0].CreatedBy.(*browserproto.BrowserCreatedByAgent)
	if !ok || creator.Number != 1 {
		t.Fatal(rows)
	}
	first.rejects(t, secondTab, "info", `{}`, "tab_not_found", "")
	if rows := second.tabs(t); len(rows) != 2 || rows[1].ID != secondTab {
		t.Fatal(rows)
	}
	first.command(t, firstTab, "close", `{}`)
	expectValue(t, callLifecycle(t, first.s, &commandproto.ConversationQuery{}), struct {
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
	failure, err := browserproto.DecodeFailureDocument(result.stderr)
	if err != nil || result.completion.ExitCode != 1 || failure.Error.Code != "browser_lost" {
		t.Fatalf("%+v %s %v", result, result.stderr, err)
	}
	if failure.Error.Details == nil || failure.Error.Details.Action == nil ||
		*failure.Error.Details.Action != "not_started" {
		t.Fatal(failure)
	}
	expectValue(t, callLifecycle(t, f.s, &commandproto.ConversationQuery{}), struct {
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
	// The service exposes no event for its retirement of a crashed browser; each
	// attempt checks whether retirement has removed the profile directory.
	for {
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
			env := tabstest.Launch(
				ctx,
				t,
				tabs.LaunchOptions{
					Executable: executable,
					Locale: commandproto.CommandLocale{
						TimeZone:  "UTC",
						Languages: []commandproto.LanguageTag{"en-US"},
					},
				},
			)
			if _, err := env.Open(ctx, "about:blank", 5*time.Second); err != nil {
				t.Fatal(err)
			}
			profile := filepath.Dir(env.DownloadDirectory())
			leader := profileOwner(t, profile)
			runtimeDirectory := profileRuntime(t, profile)
			if filepath.Dir(runtimeDirectory) != tabs.HostDirectories().Runtime {
				t.Fatal(runtimeDirectory)
			}
			for _, path := range []string{profile, runtimeDirectory} {
				info, err := os.Stat(path)
				if err != nil || info.Mode().Perm() != 0o700 {
					t.Fatalf("%s %v %v", path, info, err)
				}
			}
			owned := append(processGroup(t, leader), markedBrowserProcesses(t, runtimeDirectory)...)
			slices.Sort(owned)
			owned = slices.Compact(owned)
			if len(owned) < 2 {
				t.Fatal("Chrome has no helpers")
			}
			if pgid, err := syscall.Getpgid(leader); err != nil || pgid != leader {
				t.Fatalf("Chrome process group %d: %v", pgid, err)
			}
			injected := &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "injected work failure"}
			result := func() (err error) {
				defer func() {
					err = errors.Join(err, env.Close(context.Background()))
				}()
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
				var failure *cdp.BrowserError
				if !errors.As(result, &failure) ||
					failure.Kind != cdp.KindClosed && failure.Kind != cdp.KindConnection {
					t.Fatalf("crash error: %v", result)
				}
			}
			if group := processGroup(t, leader); len(group) != 0 {
				t.Fatal(group)
			}
			for _, pid := range owned {
				if err := syscall.Kill(pid, 0); !errors.Is(err, syscall.ESRCH) {
					t.Fatalf("Chrome helper %d survived: %v", pid, err)
				}
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
				var owned []int
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
					for _, pid := range owned {
						if err := syscall.Kill(pid, 0); !errors.Is(err, syscall.ESRCH) {
							t.Errorf("helper %d survived: %v", pid, err)
							return
						}
					}
					fmt.Println("retired after deliberate assertion: " + harness)
				})
				if harness == "service" {
					f := chromeFixture(t)
					f.open(t, "repairs.html")
					profile = filepath.Dir(f.environment(t).DownloadDirectory())
				} else {
					env := tabstest.Launch(
						t.Context(),
						t,
						tabs.LaunchOptions{
							Executable: os.Getenv("DEMI_TEST_CHROME"),
							Locale: commandproto.CommandLocale{
								TimeZone:  "UTC",
								Languages: []commandproto.LanguageTag{"en-US"},
							},
						},
					)
					if _, err := env.Open(t.Context(), "about:blank", 5*time.Second); err != nil {
						t.Fatal(err)
					}
					profile = filepath.Dir(env.DownloadDirectory())
				}
				leader = profileOwner(t, profile)
				owned = append(processGroup(t, leader), markedBrowserProcesses(t, profileRuntime(t, profile))...)
				slices.Sort(owned)
				owned = slices.Compact(owned)
				if len(owned) < 2 {
					t.Fatal("Chrome has no helpers")
				}
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
	child := exec.CommandContext(
		t.Context(),
		executable,
		"-test.run=^TestFixtureAssertionsRetireChromeAndProfilesBeforeFailureReturns$",
		"-test.count=1",
		"-test.v",
	)
	child.Env = append(os.Environ(), "DEMI_BROWSER_TEST_ASSERTION=1")
	output, err := child.CombinedOutput()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 1 {
		t.Fatalf("child %s: %v", output, err)
	}
	if strings.Count(string(output), "deliberate fixture assertion") != 2 {
		t.Fatalf("did not observe both intended assertions: %s", output)
	}
	for _, harness := range []string{"direct", "service"} {
		if !strings.Contains(string(output), "retired after deliberate assertion: "+harness) {
			t.Fatal(string(output))
		}
	}
}
