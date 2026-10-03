package edge

import (
	"context"
	"net"
	"sync"
	"time"
)

// activity records bytes moving on a visitor connection. A watch closes the
// connection on revocation or inactivity and its owner always joins it.
type activity struct {
	net.Conn
	mu   sync.Mutex
	last time.Time
}

func newActivity(conn net.Conn) *activity { return &activity{Conn: conn, last: time.Now()} }
func (a *activity) touch() {
	a.mu.Lock()
	a.last = time.Now()
	a.mu.Unlock()
}
func (a *activity) Read(p []byte) (int, error) {
	n, err := a.Conn.Read(p)
	if n > 0 {
		a.touch()
	}
	return n, err
}
func (a *activity) Write(p []byte) (int, error) {
	n, err := a.Conn.Write(p)
	if n > 0 {
		a.touch()
	}
	return n, err
}
func (a *activity) watch(ctx context.Context, limit time.Duration) func() {
	a.touch()
	canceled, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		timer := time.NewTimer(limit)
		defer timer.Stop()
		for {
			select {
			case <-canceled.Done():
				return
			case <-ctx.Done():
				_ = a.Close()
				return
			case <-timer.C:
				a.mu.Lock()
				remaining := time.Until(a.last.Add(limit))
				a.mu.Unlock()
				if remaining <= 0 {
					_ = a.Close()
					return
				}
				timer.Reset(remaining)
			}
		}
	}()
	return func() {
		cancel()
		<-done
	}
}
