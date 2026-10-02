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

func (p *credentialLifetime) Retrieve(ctx context.Context) (aws.Credentials, error) {
	p.mu.Lock()
	if p.closing {
		p.mu.Unlock()
		return aws.Credentials{}, context.Canceled
	}
	p.pending.Add(1)
	p.mu.Unlock()
	defer p.pending.Done()
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(p.ctx, cancel)
	defer stop()
	defer cancel()
	return p.provider.Retrieve(ctx)
}

// close cancels and joins refreshes before the bucket releases its transport.
func (p *credentialLifetime) close() {
	p.mu.Lock()
	p.closing = true
	p.mu.Unlock()
	p.cancel()
	p.pending.Wait()
}

// HandleFailToRefresh retains the SDK provider's refresh-failure policy.
func (p *credentialLifetime) HandleFailToRefresh(ctx context.Context, previous aws.Credentials, err error) (aws.Credentials, error) {
	if strategy, ok := p.provider.(aws.HandleFailRefreshCredentialsCacheStrategy); ok {
		return strategy.HandleFailToRefresh(ctx, previous, err)
	}
	return aws.Credentials{}, err
}

// AdjustExpiresBy retains the SDK provider's expiration adjustment policy.
func (p *credentialLifetime) AdjustExpiresBy(value aws.Credentials, duration time.Duration) (aws.Credentials, error) {
	if strategy, ok := p.provider.(aws.AdjustExpiresByCredentialsCacheStrategy); ok {
		return strategy.AdjustExpiresBy(value, duration)
	}
	if value.CanExpire {
		value.Expires = value.Expires.Add(duration)
	}
	return value, nil
}
