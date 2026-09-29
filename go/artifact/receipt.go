package artifact

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
)

// ReceiptFile is the receipt's file name inside an installation directory: what
// an installation records beside its files about what it verified, so a later
// process can trust the files without downloading them again.
const ReceiptFile = "receipt.json"

// WriteReceipt durably records receipt, the caller's document, in directory. The
// caller encodes it with its receipt type, which checks it.
func WriteReceipt(directory string, receipt []byte) error {
	return PublishBytes(filepath.Join(directory, ReceiptFile), receipt, Publication{
		Mode:        Replace,
		Permissions: DefaultPermissions,
		Durable:     true,
	})
}

// ReadReceipt returns the receipt's bytes, or nil when directory has none. The
// caller decodes them with its receipt type, which checks them.
func ReadReceipt(directory string) ([]byte, error) {
	bytes, err := os.ReadFile(filepath.Join(directory, ReceiptFile))
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	return bytes, err
}
