package artifacts

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"hash"
	"io"
	"os"
)

// Digest describes the verified bytes, before any HTTP content coding.
type Digest struct {
	Size   uint64
	SHA256 string
}

// ErrDigest means that the bytes do not match their declared SHA-256.
var ErrDigest = errors.New("the artifact does not match its declared SHA-256")

// TooLargeError means the artifact exceeded its allowed size.
type TooLargeError struct{ Declared uint64 }

func (e *TooLargeError) Error() string {
	return fmt.Sprintf("the artifact has more than its declared %d bytes", e.Declared)
}

// SizeError reports a short body or a contradictory declared length.
type SizeError struct{ Declared, Actual uint64 }

func (e *SizeError) Error() string {
	return fmt.Sprintf("the artifact has %d bytes, not the declared %d", e.Actual, e.Declared)
}

type measure struct {
	hash        hash.Hash
	size, limit uint64
}

func newMeasure(limit uint64) *measure { return &measure{hash: sha256.New(), limit: limit} }
func (m *measure) update(b []byte) error {
	if uint64(len(b)) > m.limit-m.size {
		return &TooLargeError{m.limit}
	}
	m.size += uint64(len(b))
	_, _ = m.hash.Write(b) // hash.Hash.Write never returns an error.
	return nil
}
func (m *measure) finish() Digest { return Digest{m.size, hex.EncodeToString(m.hash.Sum(nil))} }

// Verifier checks artifact chunks before they are kept.
type Verifier struct {
	expected Digest
	measure  *measure
}

// NewVerifier starts checking against expected.
func NewVerifier(expected Digest) *Verifier { return &Verifier{expected, newMeasure(expected.Size)} }

// Update rejects a chunk that would exceed the declared size.
func (v *Verifier) Update(b []byte) error { return v.measure.update(b) }

// Finish checks the complete size and hash.
func (v *Verifier) Finish() error {
	found := v.measure.finish()
	if found.Size != v.expected.Size {
		return &SizeError{v.expected.Size, found.Size}
	}
	if found.SHA256 != v.expected.SHA256 {
		return ErrDigest
	}
	return nil
}

// transfer streams artifact chunks, checking before each write. The caller
// owns input/output and must unblock their IO when ctx is cancelled; ordinary
// files and in-memory readers need no additional cancellation mechanism.
func transfer(ctx context.Context, input io.Reader, output io.Writer, check func([]byte) error) error {
	buffer := make([]byte, 64*1024)
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		n, err := input.Read(buffer)
		if n > 0 {
			if e := ctx.Err(); e != nil {
				return e
			}
			if check != nil {
				if e := check(buffer[:n]); e != nil {
					return e
				}
			}
			written, e := output.Write(buffer[:n])
			if e != nil {
				return e
			}
			if written != n {
				return io.ErrShortWrite
			}
		}
		if errors.Is(err, io.EOF) {
			return ctx.Err()
		}
		if err != nil {
			return err
		}
	}
}

// Copy verifies a local artifact while copying. The caller owns and unblocks
// input and output on cancellation if they can block indefinitely.
func Copy(ctx context.Context, input io.Reader, expected Digest, output io.Writer) error {
	v := NewVerifier(expected)
	if err := transfer(ctx, input, output, v.Update); err != nil {
		return err
	}
	return v.Finish()
}

// DigestFile measures a file, stopping as soon as it exceeds limit.
func DigestFile(ctx context.Context, path string, limit uint64) (Digest, error) {
	f, err := os.Open(path)
	if err != nil {
		return Digest{}, err
	}
	defer func() { _ = f.Close() }() // Read-only file; no buffered writes to lose.
	m := newMeasure(limit)
	if err := transfer(ctx, f, io.Discard, m.update); err != nil {
		return Digest{}, err
	}
	return m.finish(), nil
}
