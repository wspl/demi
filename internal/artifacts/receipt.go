package artifacts

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
)

//go:generate go run github.com/wspl/demi/tools/contractgen .

// ReceiptFile is the receipt's name inside an installation.
const ReceiptFile = "receipt.json"

// +demi:root direction=receive
// receipt names the archive and the verified entry it installed.
type receipt struct {
	ArchiveHash string `json:"archiveHash"`
	EntryHash   string `json:"entryHash"`
}

// WriteReceipt durably publishes a receipt whose contract owner supplies its
// generated JSON marshaler. It does not declare the caller's receipt shape.
func WriteReceipt(ctx context.Context, directory string, receipt json.Marshaler) error {
	data, err := receipt.MarshalJSON()
	if err != nil {
		return err
	}
	return PublishBytes(ctx, filepath.Join(directory, ReceiptFile), data, Publication{Mode: Replace, Durable: true})
}

// ReadReceipt returns nil when absent. Callers decode through their generated codec.
func ReadReceipt(ctx context.Context, directory string) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(filepath.Join(directory, ReceiptFile))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	return data, err
}
