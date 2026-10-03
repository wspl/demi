package blobs

import (
	"context"
	"sync"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
)

// credentialLifetime owns credential refreshes, which the SDK cache deliberately
// lets outlive a canceled request. Bucket shutdown cancels and joins their IO.
type credentialLifetime struct {
	provider aws.CredentialsProvider
	ctx      context.Context
	cancel   context.CancelFunc
	// mu serializes refresh admission with shutdown; waits happen after unlock.
	mu      sync.Mutex
	closing bool
	pending sync.WaitGroup
}

// Retrieve owns a credential refresh until completion or bucket shutdown.
func (l *credentialLifetime) Retrieve(ctx context.Context) (aws.Credentials, error) {
	l.mu.Lock()
	if l.closing {
		l.mu.Unlock()
		return aws.Credentials{}, context.Canceled
	}
	l.pending.Add(1)
	l.mu.Unlock()
	defer l.pending.Done()
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(l.ctx, cancel)
	defer stop()
	defer cancel()
	return l.provider.Retrieve(ctx)
}

// close cancels and joins refreshes before the bucket releases its transport.
func (l *credentialLifetime) close() {
	l.mu.Lock()
	l.closing = true
	l.mu.Unlock()
	l.cancel()
	l.pending.Wait()
}

// HandleFailToRefresh retains the SDK provider's refresh-failure policy.
func (l *credentialLifetime) HandleFailToRefresh(
	ctx context.Context,
	previous aws.Credentials,
	err error,
) (aws.Credentials, error) {
	if strategy, ok := l.provider.(aws.HandleFailRefreshCredentialsCacheStrategy); ok {
		return strategy.HandleFailToRefresh(ctx, previous, err)
	}
	return aws.Credentials{}, err
}

// AdjustExpiresBy retains the SDK provider's expiration adjustment policy.
func (l *credentialLifetime) AdjustExpiresBy(value aws.Credentials, duration time.Duration) (aws.Credentials, error) {
	if strategy, ok := l.provider.(aws.AdjustExpiresByCredentialsCacheStrategy); ok {
		return strategy.AdjustExpiresBy(value, duration)
	}
	if value.CanExpire {
		value.Expires = value.Expires.Add(duration)
	}
	return value, nil
}
