package database

import (
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// OutputRow is what a conversation holds of an ended command's output.
//
//sumtype:decl
type OutputRow interface{ outputRow() }

// OutputStored holds its blob and trailing bytes the backend does not have.
type OutputStored struct {
	Blob    types.BlobRef
	Missing *host.Missing
}

func (*OutputStored) outputRow() {}

// OutputNotStored records why output was not stored.
type OutputNotStored struct{ Reason string }

func (*OutputNotStored) outputRow() {}

// OutputRemoved records when retention removed the output.
type OutputRemoved struct{ At types.Timestamp }

func (*OutputRemoved) outputRow() {}
