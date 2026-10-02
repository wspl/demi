package blobs

// Error reports why an operation on the object store failed.
// Err retains the driver's error for errors.Is, errors.As and gcerrors.Code.
type Error struct {
	Err error
}

// Error describes the object-store failure.
func (e *Error) Error() string {
	panic("not written: b-blobs")
}

// Unwrap returns the underlying failure.
func (e *Error) Unwrap() error {
	panic("not written: b-blobs")
}

// CorruptError reports an object's metadata outside its type, such as a write
// time out of range; nothing repairs it.
type CorruptError struct {
	Location string
	Field    string
	Reason   string
}

// Error identifies the object and its invalid metadata.
func (e *CorruptError) Error() string {
	panic("not written: b-blobs")
}

// ConfigError reports why the S3 configuration cannot be read or used.
// Err retains the filesystem or validation failure for errors.Is and errors.As.
type ConfigError struct {
	Err error
}

// Error describes the configuration failure.
func (e *ConfigError) Error() string {
	panic("not written: b-blobs")
}

// Unwrap returns the underlying filesystem or validation failure.
func (e *ConfigError) Unwrap() error {
	panic("not written: b-blobs")
}
