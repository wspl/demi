package database

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// PluginValue is a stored value and its revision.
type PluginValue struct {
	Document json.RawMessage
	Revision uint64
}

// ValueWrite is a write of a plugin value.
type ValueWrite struct {
	User     webapi.UserID
	Plugin   string
	Key      string
	Document json.RawMessage
	// The revision the write read; none for a value that does not exist
	// yet.
	Revision *uint64
	// The blobs the value names.
	Blobs []core.BlobRef
}
