package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// OutputRow is what a conversation holds of an ended command's output.
//
//sumtype:decl
type OutputRow interface{ outputRow() }

// OutputStored holds its blob and trailing bytes the backend does not have.
type OutputStored struct {
	Blob    core.BlobRef
	Missing *host.Missing
}

func (*OutputStored) outputRow() {}

// OutputNotStored records why output was not stored.
type OutputNotStored struct{ Reason string }

func (*OutputNotStored) outputRow() {}

// OutputRemoved records when retention removed the output.
type OutputRemoved struct{ At core.Timestamp }

func (*OutputRemoved) outputRow() {}
