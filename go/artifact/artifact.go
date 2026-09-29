// Package artifact holds verified bytes (docs/architecture/crates-and-packages.md
// § artifact): downloads over HTTPS (plain HTTP too for a caller whose digest
// came over a connection it trusts) with a declared size and SHA-256, and
// measured ones for a release being prepared, digests, durable atomic
// publication, release publication, the install lock between processes,
// install receipts and archive installation. Callers name the location, size
// and digest they expect; nothing here chooses what to install.
//
// A failure that is the operating system's (a file that cannot be opened, a
// disk that is full) is returned as it is, so a caller that waits out a lack
// of open files finds it with [errors.Is]. The other failures are the types of
// errors.go. A cancelled context ends an operation with the context's error.
package artifact

// A Digest is a file's size and its SHA-256 in lowercase hex.
type Digest struct {
	Size   uint64
	SHA256 string
}
