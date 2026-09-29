package artifact

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"hash"
	"io"
	"os"
)

// chunkBytes is how many bytes a copy or a digest reads at once.
const chunkBytes = 64 * 1024

// A measure counts and hashes bytes as they arrive; bytes past its limit fail at
// once.
type measure struct {
	hash  hash.Hash
	size  uint64
	limit uint64
}

func newMeasure(limit uint64) *measure {
	return &measure{hash: sha256.New(), limit: limit}
}

// Write counts and hashes the next chunk, or fails with a [*TooLargeError]
// without counting it when it takes the bytes seen past the limit.
func (m *measure) Write(chunk []byte) (int, error) {
	if uint64(len(chunk)) > m.limit-m.size {
		return 0, &TooLargeError{Declared: m.limit}
	}
	m.size += uint64(len(chunk))
	m.hash.Write(chunk)
	return len(chunk), nil
}

// digest returns the size and SHA-256 of the bytes seen.
func (m *measure) digest() Digest {
	return Digest{Size: m.size, SHA256: hex.EncodeToString(m.hash.Sum(nil))}
}

// A Verifier checks bytes against a declared digest as they arrive, so a
// download that grows past its size stops at once. It is an [io.Writer]: bytes
// past the declared size fail with a [*TooLargeError].
type Verifier struct {
	expected Digest
	measure  *measure
}

// NewVerifier returns a verifier of bytes against expected.
func NewVerifier(expected Digest) *Verifier {
	return &Verifier{expected: expected, measure: newMeasure(expected.Size)}
}

// Write counts and hashes the next chunk.
func (v *Verifier) Write(chunk []byte) (int, error) {
	return v.measure.Write(chunk)
}

// Finish reports whether the bytes seen are exactly the declared ones: a
// [*SizeError] when there are fewer, [ErrDigest] when they differ.
func (v *Verifier) Finish() error {
	seen := v.measure.digest()
	if seen.Size != v.expected.Size {
		return &SizeError{Declared: v.expected.Size, Actual: seen.Size}
	}
	if seen.SHA256 != v.expected.SHA256 {
		return ErrDigest
	}
	return nil
}

// DigestFile hashes the file at path; a file longer than limit bytes fails with
// a [*TooLargeError] without being read to its end. Cancelling ctx stops it
// before its next read.
func DigestFile(ctx context.Context, path string, limit uint64) (Digest, error) {
	file, err := os.Open(path)
	if err != nil {
		return Digest{}, err
	}
	defer file.Close()
	m := newMeasure(limit)
	if err := copyChunks(ctx, file, m, nil); err != nil {
		return Digest{}, err
	}
	return m.digest(), nil
}

// copyChunks reads src to its end into dst, in chunks, and stops before the
// next read once ctx is done. A failure of a read is passed through wrap when
// there is one, so that a caller tells it from a failure of the write.
func copyChunks(ctx context.Context, src io.Reader, dst io.Writer, wrap func(error) error) error {
	buffer := make([]byte, chunkBytes)
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		count, err := src.Read(buffer)
		if count > 0 {
			if _, err := dst.Write(buffer[:count]); err != nil {
				return err
			}
		}
		switch {
		case err == io.EOF:
			return nil
		case err != nil && wrap != nil:
			return wrap(err)
		case err != nil:
			return err
		}
	}
}
