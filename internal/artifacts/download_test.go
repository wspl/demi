package artifacts

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"testing"
	"testing/synctest"
	"time"
)

// The in-memory connection lets synctest prove the full idle timeout and a
// longer, progressing download without spending wall time or starting a server.
func TestDownloadIdleTimeoutResetsOnProgress(t *testing.T) {
	for _, stalled := range []bool{false, true} {
		t.Run(fmt.Sprint(stalled), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				clientSide, serverSide := net.Pipe()
				defer func() { _ = clientSide.Close() }() // Also unblocks the transport on failure.
				defer func() { _ = serverSide.Close() }() // Also unblocks the fixture on failure.
				transport := &http.Transport{DialContext: func(context.Context, string, string) (net.Conn, error) { return &progressConn{Conn: clientSide}, nil }}
				client := &Client{http: &http.Client{Transport: transport}, allowHTTP: true}
				defer client.Close()
				done := make(chan error, 1)
				go func() {
					_, err := http.ReadRequest(bufio.NewReader(serverSide))
					if err != nil {
						done <- err
						return
					}
					_, err = io.WriteString(serverSide, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n")
					if err != nil {
						done <- err
						return
					}
					if stalled {
						time.Sleep(61 * time.Second)
					} else {
						for range 2 {
							time.Sleep(59 * time.Second)
							if _, err = io.WriteString(serverSide, "a"); err != nil {
								done <- err
								return
							}
						}
					}
					done <- serverSide.Close()
				}()
				start := time.Now()
				found, err := DownloadMeasured(t.Context(), client, "http://artifact.test/file", 2, io.Discard)
				if stalled {
					var timeout net.Error
					if !errors.As(err, &timeout) || !timeout.Timeout() {
						t.Errorf("want idle timeout, got %v", err)
					}
					if elapsed := time.Since(start); elapsed != 60*time.Second {
						t.Errorf("idle timeout took %s", elapsed)
					}
				} else {
					if err != nil || found.Size != 2 {
						t.Errorf("progressing download: %v, %v", found, err)
					}
					if elapsed := time.Since(start); elapsed != 118*time.Second {
						t.Errorf("progressing download took %s", elapsed)
					}
				}
				if err := <-done; err != nil {
					t.Fatal(err)
				}
			})
		})
	}
}
