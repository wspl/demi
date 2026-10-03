package remotehosttest

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// One built runner, restarted once against a local rejecting endpoint. Requests
// establish startup; Stop joins each child and its readers, with no sleep.
func TestOwnedRunnerBorrowsDirectoryAndJoinsAcrossRestart(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
	defer cancel()
	arrived := make(chan struct{}, 2)
	vendor := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
		select {
		case arrived <- struct{}{}:
		default:
		}
	}))
	defer vendor.Close()
	directory, err := runnerTempDir(t)
	if err != nil {
		t.Fatal(err)
	}
	runner, err := StartOwnedRunnerProcess(ctx, directory, vendor.URL, DefaultRunnerProcessOptions())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := runner.Stop(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	for i := 0; i < 2; i++ {
		select {
		case <-arrived:
		case <-ctx.Done():
			t.Fatalf("runner did not connect: %v\n%s", ctx.Err(), runner.Output())
		}
		stopped, stop := context.WithCancel(ctx)
		stop()
		if err := runner.Stop(stopped); err != nil {
			t.Fatal(err)
		}
		if runner.Running() {
			t.Fatal("Stop returned with a running child")
		}
		for _, path := range []string{directory, runner.Home(), runner.StateDir()} {
			if _, err := os.Stat(path); err != nil {
				t.Fatalf("Stop removed borrowed directory: %v", err)
			}
		}
		if i == 0 {
			if err := runner.StartAgain(ctx); err != nil {
				t.Fatal(err)
			}
		}
	}
}
