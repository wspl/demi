package backendtest

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// The program starts one real runner at a local WebSocket, then must join it on
// EOF or cancellation. All synchronization is socket/pipe events; 20s guards hangs.
func TestScriptedMachinesEndsWithInputOrCancellation(t *testing.T) {
	for _, ending := range []string{"eof", "cancel"} {
		t.Run(ending, func(t *testing.T) {
			guard, stopGuard := context.WithTimeout(t.Context(), 20*time.Second)
			defer stopGuard()
			ctx, cancel := context.WithCancel(guard)
			defer cancel()
			connected, disconnected := make(chan struct{}), make(chan struct{})
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				socket, err := websocket.Accept(w, r, nil)
				if err != nil {
					t.Error(err)
					return
				}
				defer func() { _ = socket.CloseNow() }() // The read owns disconnection diagnostics.
				close(connected)
				defer close(disconnected)
				for {
					if _, _, err := socket.Read(r.Context()); err != nil {
						return
					}
				}
			}))
			defer server.Close()
			input, writeInput := io.Pipe()
			output, writeOutput := io.Pipe()
			var diagnostics bytes.Buffer
			done := make(chan struct{})
			var code int
			artifacts := t.TempDir()
			go func() {
				code = ScriptedMachines(ctx, []string{"--artifacts", artifacts}, input, writeOutput, &diagnostics)
				_ = writeOutput.Close() // Wake a first-line reader even after an early failure.
				close(done)
			}()
			defer func() {
				cancel()
				_ = writeInput.Close()
				_ = output.Close()
				<-done // Join the program even after a fatal assertion.
			}()
			path, err := bufio.NewReader(output).ReadString('\n')
			if err != nil {
				t.Fatal(err)
			}
			path = strings.TrimSuffix(path, "\n")
			socket, err := (&net.Dialer{}).DialContext(ctx, "unix", path)
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = socket.Close() }()
			backend, err := runnerwire.ParseBackendURL(server.URL)
			if err != nil {
				t.Fatal(err)
			}
			token, err := runnerwire.ParseDeviceToken(strings.Repeat("a", 64))
			if err != nil {
				t.Fatal(err)
			}
			line, err := machinewire.EncodeLine(
				machinewire.MachineRequest{
					ID: "wake",
					Call: &machinewire.Wake{
						Params: machinewire.WakeParams{
							DeviceID: "device",
							Boot:     runnerwire.ManagedBoot{BackendURL: backend, DeviceToken: token},
						},
					},
				},
			)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := socket.Write(line); err != nil {
				t.Fatal(err)
			}
			response, err := bufio.NewReader(socket).ReadBytes('\n')
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := machinewire.DecodeResponse(response)
			if err != nil {
				t.Fatal(err)
			}
			if _, ok := decoded.(*machinewire.OK); !ok {
				t.Fatalf("wake: %v", decoded)
			}
			select {
			case <-connected:
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
			if ending == "eof" {
				if err := writeInput.Close(); err != nil {
					t.Fatal(err)
				}
			} else {
				cancel()
			}
			<-done
			if code != 0 {
				t.Fatalf("exit %d: %s", code, diagnostics.String())
			}
			select {
			case <-disconnected:
			case <-guard.Done():
				t.Fatal(guard.Err())
			}
			if _, err := os.Stat(filepath.Dir(path)); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("manager directory remains: %v", err)
			}
		})
	}
}
