package backendtest_test

import (
	"bytes"
	"context"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/procgroup"
)

// Cost: one backend, about a second.
func TestShutdownClosesTheListener(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t).Start()
	url := b.URL
	b.Stop()
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 5 * time.Second}
	if response, err := client.Get(url + "/api/setup"); err == nil {
		_ = response.Body.Close()
		t.Fatal("the backend still answers after shutdown")
	}
}

// startWith runs the backend program with exactly these variables and answers its
// output and whether it succeeded.
func startWith(t *testing.T, variables [][2]string) (string, bool) {
	t.Helper()
	// A backend that started would serve until stopped: a refusal ends by itself.
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, backendtest.Program(t, "demi-backend"))
	command.Env = nil
	for _, variable := range variables {
		command.Env = append(command.Env, variable[0]+"="+variable[1])
	}
	var output bytes.Buffer
	command.Stdout = &output
	command.Stderr = &output
	if err := procgroup.Start(command); err != nil {
		t.Fatal(err)
	}
	err := command.Wait()
	return output.String(), err == nil
}

// requiredVariables are the variables the backend needs to get as far as its
// configuration checks.
func requiredVariables() [][2]string {
	return [][2]string{
		{"DEMI_INSTANCE_MODE", "shared"},
		{"DEMI_BACKEND_PUBLIC_URL", "http://127.0.0.1:3271"},
		{"DEMI_MACHINES_SOCKET", "/nonexistent/demi-machines.sock"},
		{"DEMI_NATIVE_CONFIG", "/nonexistent/native.json"},
	}
}

func refusedNaming(t *testing.T, output string, succeeded bool, variable string) {
	t.Helper()
	if succeeded || !strings.Contains(output, variable) {
		t.Fatalf("the backend (succeeded %v) does not name %s:\n%s", succeeded, variable, output)
	}
}

// Cost: one process start, about 0.1 s.
func TestAPortThatIsNotANumberStopsStartupNamingTheVariable(t *testing.T) {
	t.Parallel()
	output, succeeded := startWith(t, append(requiredVariables(), [2]string{"DEMI_BACKEND_PORT", "abc"}))
	refusedNaming(t, output, succeeded, "DEMI_BACKEND_PORT")
}

// Cost: one process start, about 0.1 s.
func TestAMissingPublicURLStopsStartupNamingTheVariable(t *testing.T) {
	t.Parallel()
	var variables [][2]string
	for _, variable := range requiredVariables() {
		if variable[0] != "DEMI_BACKEND_PUBLIC_URL" {
			variables = append(variables, variable)
		}
	}
	output, succeeded := startWith(t, variables)
	refusedNaming(t, output, succeeded, "DEMI_BACKEND_PUBLIC_URL")
}

// Cost: one process start, about 0.1 s.
func TestAMalformedInstanceSecretStopsStartupWithoutShowingIt(t *testing.T) {
	t.Parallel()
	data, err := os.MkdirTemp("", "demi-backend-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(data) })
	variables := append(requiredVariables(),
		[2]string{"DEMI_BACKEND_DATA", data},
		[2]string{"DEMI_INSTANCE_SECRET", "not-a-hex-secret-value"},
	)
	output, succeeded := startWith(t, variables)
	refusedNaming(t, output, succeeded, "DEMI_INSTANCE_SECRET")
	if strings.Contains(output, "not-a-hex-secret-value") {
		t.Fatalf("the backend shows the secret:\n%s", output)
	}
}
