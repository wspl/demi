package backendtest

import (
	"context"
	"errors"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/backend/accounts"
)

// Mailbox captures verification mail, or refuses it while failing is set.
// Its zero value is ready; all methods support concurrent use.
type Mailbox struct {
	mu      sync.Mutex
	failing bool
	sent    []accounts.VerificationMail
}

// SetFailing makes subsequent deliveries fail with the fixture transport error.
func (m *Mailbox) SetFailing(failing bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.failing = failing
}

// SendVerification captures mail or returns the scripted transport failure.
func (m *Mailbox) SendVerification(_ context.Context, mail accounts.VerificationMail) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failing {
		return errors.New("the mail transport failed")
	}
	m.sent = append(m.sent, mail)
	return nil
}

// Sent returns a snapshot of delivered verification messages, in send order.
func (m *Mailbox) Sent() []accounts.VerificationMail {
	m.mu.Lock()
	defer m.mu.Unlock()
	return slices.Clone(m.sent)
}

var _ accounts.AccountMail = (*Mailbox)(nil)
