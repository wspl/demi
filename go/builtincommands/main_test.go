package builtincommands_test

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/builtincommands"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// runAsProgram is the variable that makes the test binary run as the program
// demi-commands, so that a test can start the program without building it.
const runAsProgram = "DEMI_TEST_RUN_AS_DEMI_COMMANDS"

func TestMain(m *testing.M) {
	if os.Getenv(runAsProgram) != "" {
		builtincommands.Main()
	}
	os.Exit(m.Run())
}

func TestTheProgramServesTheFileOperationsAndExitsWithStatusZeroAfterAShutdown(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	process := servicetest.StartProcess(ctx, t, os.Args[0], []string{"--command-service"}, []string{runAsProgram + "=1"})
	info, err := process.Client.Info(ctx)
	if err != nil || len(info.Operations) != 52 || info.Operations[0] != "file.read" {
		t.Fatalf("the program lists %v, %v", info.Operations, err)
	}
	dir := t.TempDir()
	// A recording that cannot begin is a diagnostic on standard error, which
	// the runner drains into the Host's log; standard output carries protocol
	// bytes only, so the operation goes on.
	if err := os.Mkdir(filepath.Join(dir, "edits.lock"), 0o755); err != nil {
		t.Fatal(err)
	}
	stream, err := process.Client.Invoke(ctx, commandservice.Invocation{
		Operation: "file.create", InvocationID: "create",
		Context: commandservice.CommandContext{Conversation: "c", Caller: commandservice.UserCaller{}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}}},
		Args:    jsontext.Value(`{"path":"a.txt","content":"a"}`), Cwd: dir, Env: map[string]string{},
		Edits: &commandservice.EditContext{Directory: filepath.Join(dir, "changes"), Lock: filepath.Join(dir, "edits.lock")},
	})
	if err != nil {
		t.Fatal(err)
	}
	var stdout bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &bytes.Buffer{})
	if err != nil || completion.ExitCode != 0 || stdout.String() != "Created a.txt\n" {
		t.Fatalf("%+v, %v, %q", completion, err, stdout.String())
	}
	if err := process.Client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	state, err := process.Wait(ctx)
	if err != nil || state.ExitCode() != 0 {
		t.Errorf("the program ended with %v, %v", state, err)
	}
	// The diagnostic is one line that names what failed, and has no time of its
	// own: the log adds its own.
	line := strings.TrimSpace(process.Stderr())
	if !strings.HasPrefix(line, `level=WARN msg="edit recording failed" error=`) || strings.Contains(line, "\n") || strings.Contains(line, "time=") {
		t.Errorf("standard error = %q", process.Stderr())
	}
}

func TestTheProgramStartedWithoutItsFlagSaysHowToStartItAndExitsWithStatusTwo(t *testing.T) {
	for _, args := range [][]string{nil, {"--other"}, {"serve", "--command-service"}} {
		var stdout, stderr bytes.Buffer
		cmd := exec.Command(os.Args[0], args...)
		cmd.Env = append(os.Environ(), runAsProgram+"=1")
		cmd.Stdout, cmd.Stderr = &stdout, &stderr
		err := cmd.Run()
		exit, ok := err.(*exec.ExitError)
		if !ok || exit.ExitCode() != 2 {
			t.Errorf("%v: %v, want exit status 2", args, err)
		}
		if stdout.Len() != 0 || stderr.String() != "Usage: demi-commands --command-service\n" {
			t.Errorf("%v: standard output %q, standard error %q", args, stdout.String(), stderr.String())
		}
	}
}
