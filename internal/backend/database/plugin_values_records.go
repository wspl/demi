package database

import (
	"encoding/json"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// PluginValue is a stored value and its revision.
type PluginValue struct {
	Document json.RawMessage
	Revision uint64
}

// ValueWrite is a write of a plugin value.
type ValueWrite struct {
	User     webapiproto.UserID
	Plugin   string
	Key      string
	Document json.RawMessage
	// The revision the write read; none for a value that does not exist
	// yet.
	Revision *uint64
	// The blobs the value names.
	Blobs []types.BlobRef
}
