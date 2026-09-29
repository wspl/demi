//go:build unix

package claude_test

import (
	"encoding/json/v2"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/go/commandservice/servicetest"
)

// The program at its boundary (docs/providers/claude-code.md § The package): the
// command service it serves on its standard input and output, as a runner starts
// it, answers each invocation with one document and says a failure in its
// completion too. The installer's rules, which download from a fixture server,
// are tested beside it in installer_test.go. HOME names the user's root on Unix
// only.
func TestTheProgramServesItsOperationsAsARunnerStartsIt(t *testing.T) {
	home := t.TempDir()
	preinstall(t, filepath.Join(home, ".demi", "claude"), "2.1.278", body)
	path := filepath.Join(home, ".demi", "claude", "2.1.278", "claude")
	// A test binary built with the race detector waits a second before it exits,
	// to let other threads report races; the service has none to wait for.
	service := servicetest.StartProcess(t.Context(), t, os.Args[0], []string{"--command-service"},
		[]string{asProgram + "=1", "HOME=" + home, "GORACE=atexit_sleep_ms=0"})
	ctx := t.Context()
	info, err := service.Client.Info(ctx)
	if err != nil || len(info.Operations) != 2 || info.Operations[0] != "claude.ensure" || info.Operations[1] != "claude.status" {
		t.Fatalf("info = %+v, %v", info, err)
	}

	// The installation is in place, so the answer needs no download.
	document, completion := invoke(ctx, t, service.Client, "claude.ensure", record(t, "2.1.278", platform(t), "https://downloads.example.test/claude", len(body), sha256Hex(body)))
	if want := `{"ok":true,"version":"2.1.278","path":` + jsonString(t, path) + `}`; document != want || completion.ExitCode != 0 || completion.Error != nil {
		t.Errorf("ensure: %s, %+v; want %s", document, completion, want)
	}
	document, completion = invoke(ctx, t, service.Client, "claude.status", nil)
	want := `{"ok":true,"platform":"` + platform(t) + `","installed":[{"version":"2.1.278","path":` + jsonString(t, path) + `}]}`
	if document != want || completion.ExitCode != 0 {
		t.Errorf("status: %s, %+v; want %s", document, completion, want)
	}

	// The program downloads over HTTPS only, from any address.
	document, completion = invoke(ctx, t, service.Client, "claude.ensure", record(t, "2.1.279", platform(t), "http://127.0.0.1:9/claude", len(body), sha256Hex(body)))
	if completion.ExitCode != 1 || completion.Error == nil || completion.Error.Code != "invalid_release" {
		t.Errorf("plain HTTP: %s, %+v", document, completion)
	}
	if document, completion = invoke(ctx, t, service.Client, "claude.ensure", []byte(`{}`)); completion.ExitCode != 1 || completion.Error == nil || completion.Error.Code != "invalid_release" {
		t.Errorf("an empty record: %s, %+v", document, completion)
	}

	if err := service.Client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	state, err := service.Wait(ctx)
	if err != nil || !state.Success() {
		t.Errorf("the program ended with %v, %v", state, err)
	}
}

func jsonString(t *testing.T, s string) string {
	t.Helper()
	data, err := json.Marshal(s)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}
