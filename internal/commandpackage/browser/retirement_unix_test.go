//go:build darwin || linux

package browser_test

import (
	"bufio"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs/tabstest"
	"github.com/wspl/demi/internal/commandproto"
)

// A fixture process writes Chrome's refusal and exits at once. Repeated launches
// protect the stderr/exit race; no real Chrome or wall-time wait is needed.
func TestLaunchAsRootSaysToRunRunnerAsOrdinaryUser(t *testing.T) {
	launcher := filepath.Join(t.TempDir(), "root-chrome")
	script := "#!/bin/sh\n" +
		"echo '[1:1:0927/010848.716678:ERROR:content/browser/zygote_host/zygote_host_impl_linux.cc:102] " +
		"Running as root without --no-sandbox is not supported." +
		" See https://crbug.com/638180.' >&2\nexit 1\n"
	if err := os.WriteFile(launcher, []byte(script), 0o700); err != nil {
		t.Fatal(err)
	}
	for range 8 {
		environment, err := tabs.Launch(
			t.Context(),
			tabs.LaunchOptions{
				Executable: launcher,
				Locale:     commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
			},
			&tabstest.Numbers{},
		)
		if environment != nil {
			if cleanup := environment.Close(t.Context()); cleanup != nil {
				t.Error(cleanup)
			}
			t.Fatal("refused launch returned a browser")
		}
		var failure *cdp.BrowserError
		if !errors.As(err, &failure) || failure.Kind != cdp.KindRoot ||
			!strings.Contains(err.Error(), "run the runner as an ordinary user") {
			t.Fatalf("root refusal: %v", err)
		}
	}
}

func TestCanceledLaunchReapsHelpersBeforeRemovingProfile(t *testing.T) {
	root := t.TempDir()
	launcher := filepath.Join(root, "pending-chrome")
	record := launcher + ".record"
	if err := syscall.Mkfifo(record, 0o600); err != nil {
		t.Fatal(err)
	}
	pipe, err := os.OpenFile(record, os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := pipe.Close(); err != nil {
			t.Error(err)
		}
	}()
	script := "#!/bin/sh\nsleep 120 &\nprintf '%s\\n' \"$$\" \"$!\" \"$@\" ready > \"$0.record\"\nwait\n"
	if err := os.WriteFile(launcher, []byte(script), 0o700); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() {
		environment, err := tabs.Launch(
			ctx,
			tabs.LaunchOptions{
				Executable: launcher,
				Locale:     commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
			},
			&tabstest.Numbers{},
		)
		if environment != nil {
			t.Error("cancelled launch returned an environment")
			err = errors.Join(err, environment.Close(context.Background()))
		}
		done <- err
	}()
	t.Cleanup(func() {
		cancel()
		<-done
	})
	scanner := bufio.NewScanner(pipe)
	lines := []string{}
	for scanner.Scan() {
		if scanner.Text() == "ready" {
			break
		}
		lines = append(lines, scanner.Text())
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	if len(lines) < 3 {
		t.Fatal(lines)
	}
	cancel()
	err = <-done
	done <- err
	if !errors.Is(err, context.Canceled) && cdp.ErrorCode(err) != "cancelled" {
		t.Fatal(err)
	}
	for _, line := range lines[:2] {
		pid, err := strconv.Atoi(line)
		if err != nil {
			t.Fatal(err)
		}
		if err := syscall.Kill(pid, 0); !errors.Is(err, syscall.ESRCH) {
			t.Fatalf("launcher helper %d survived: %v", pid, err)
		}
	}
	leader, err := strconv.Atoi(lines[0])
	if err != nil {
		t.Fatal(err)
	}
	if err := syscall.Kill(-leader, 0); !errors.Is(err, syscall.ESRCH) {
		t.Fatalf("process group survived: %v", err)
	}
	foundProfile := false
	for _, line := range lines[2:] {
		if profile, ok := strings.CutPrefix(line, "--user-data-dir="); ok {
			foundProfile = true
			if _, err := os.Stat(profile); !os.IsNotExist(err) {
				t.Fatalf("profile survived: %v", err)
			}
		}
	}
	if !foundProfile {
		t.Fatal("launcher did not record a profile")
	}
}
