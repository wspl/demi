//go:build unix

package builtincommands_test

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"os"
	"path/filepath"
	"syscall"
	"testing"

	"github.com/wspl/demi/go/builtincommands"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// A read that waits for a file to say more, a pipe, ends when its invocation is
// cancelled, so the service that ends is not late (a handler that does not stop
// soon after its call ends is a fault of the service).
func TestAReadThatWaitsOnAPipeEndsWhenItsInvocationIsCancelled(t *testing.T) {
	server := servicetest.Start(t, builtincommands.New())
	dir := t.TempDir()
	fifo := filepath.Join(dir, "pipe")
	if err := syscall.Mkfifo(fifo, 0o644); err != nil {
		t.Fatal(err)
	}
	// The writer opens the pipe, which lets the reader open it, and says nothing.
	writer := make(chan *os.File, 1)
	go func() {
		file, err := os.OpenFile(fifo, os.O_WRONLY, 0)
		if err != nil {
			t.Error(err)
		}
		writer <- file
	}()
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	stream, err := server.Client.Invoke(ctx, commandservice.Invocation{
		Operation: "file.read", InvocationID: "read",
		Context: commandservice.CommandContext{Conversation: "c", Caller: commandservice.UserCaller{}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}}},
		Args:    jsontext.Value(`{"path":"pipe"}`), Cwd: dir, Env: map[string]string{},
	})
	if err != nil {
		t.Fatal(err)
	}
	file := <-writer
	defer file.Close()
	if _, err := file.WriteString("first\n"); err != nil {
		t.Fatal(err)
	}
	// What the pipe said arrives, and then the read waits.
	var seen bytes.Buffer
	for seen.String() != "first\n" {
		record, err := stream.Next()
		if err != nil {
			t.Fatal(err)
		}
		if record.Kind == commandservice.RecordStdout {
			seen.Write(record.Data)
		}
	}
	stream.Cancel()
	if err := server.Client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err := server.Wait(ctx); err != nil {
		t.Errorf("the service ended with %v; the read did not stop when it was cancelled", err)
	}
}
