package backendtest

import (
	"context"
	"net/http"
	"testing"

	"github.com/wspl/demi/go/backendtest/runnerproc"
)

// A Paired is a paired device and its real runner.
type Paired struct {
	t testing.TB
	b *Backend
	// Runner is the demi-runner process; a scenario may stop it and start it
	// again as the same device.
	Runner *runnerproc.Process
	// Device is the device as the API lists it.
	Device map[string]any
}

// ID is the device's id.
func (p *Paired) ID() string {
	id, _ := p.Device["id"].(string)
	return id
}

// Home is the runner's home directory, where its Hosts start work.
func (p *Paired) Home() string {
	return p.Runner.Home()
}

// Token is the device token the runner stored once it was claimed.
func (p *Paired) Token() string {
	p.t.Helper()
	return StoredToken(p.t, p.Runner)
}

// StoredToken waits until the runner stored its device token, and answers it.
// The backend binds the device before the runner hears its token, so the claim
// answers, and the device is online, a moment before the runner holds the
// token; a runner stopped in that moment comes back unpaired.
func StoredToken(t testing.TB, runner *runnerproc.Process) string {
	t.Helper()
	var token string
	Eventually(t, "the runner stores its token", func() bool {
		stored, ok := runner.Token()
		token = stored
		return ok
	})
	return token
}

// Devices returns the session's paired devices, as GET /api/devices lists them.
func (b *Backend) Devices(session *Session) []map[string]any {
	b.t.Helper()
	answer := b.Get("/api/devices", session).Expect(http.StatusOK)
	var devices []map[string]any
	list, _ := answer.At("devices").([]any)
	for _, device := range list {
		devices = append(devices, device.(map[string]any))
	}
	return devices
}

// Online reports whether the session's device id is online, as the device list
// says.
func (b *Backend) Online(session *Session, id string) bool {
	b.t.Helper()
	for _, device := range b.Devices(session) {
		if device["id"] == id && device["online"] == true {
			return true
		}
	}
	return false
}

// UntilOnline waits until the device's online state is online.
func (b *Backend) UntilOnline(session *Session, id string, online bool) {
	b.t.Helper()
	Eventually(b.t, "the device's online state changes", func() bool {
		return b.Online(session, id) == online
	})
}

// Pair starts a runner of a new device named name, claims it as session, and
// answers once it is online and holds its device token, so a scenario may stop
// it and start it again as the same device.
func (b *Backend) Pair(session *Session, name string) *Paired {
	b.t.Helper()
	runner, err := runnerproc.Start(Program(b.t, "demi-runner"), b.URL, b.h.Root, runnerproc.Options{Name: name})
	if err != nil {
		b.t.Fatal(err)
	}
	b.t.Cleanup(runner.Remove)
	code := PairingCode(b.t, runner, 0)
	claimed := b.Post("/api/devices/claim", session, Map{"code": code}).Expect(http.StatusCreated)
	device, _ := claimed.At("device").(map[string]any)
	paired := &Paired{t: b.t, b: b, Runner: runner, Device: device}
	b.UntilOnline(session, paired.ID(), true)
	StoredToken(b.t, runner)
	return paired
}

// StartRunner starts a runner of the real program for this backend, which waits
// to be paired unless options give it a token. It ends with the test.
func (b *Backend) StartRunner(options runnerproc.Options) *runnerproc.Process {
	b.t.Helper()
	runner, err := runnerproc.Start(Program(b.t, "demi-runner"), b.URL, b.h.Root, options)
	if err != nil {
		b.t.Fatal(err)
	}
	b.t.Cleanup(runner.Remove)
	return runner
}

// PairingCode waits for the runner's index'th pairing code.
func PairingCode(t testing.TB, runner *runnerproc.Process, index int) string {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	code, err := runner.PairingCode(ctx, index)
	if err != nil {
		t.Fatal(err)
	}
	return code
}
