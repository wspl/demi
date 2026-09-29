package artifact

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/v2"

	"github.com/wspl/demi/go/internal/wire"
)

// An archiveReceipt is what an archive installation records beside its files:
// the SHA-256 of the archive it came from and of its executable.
//
//demi:wire
type archiveReceipt struct {
	ArchiveHash    string `json:"archiveHash"`
	ExecutableHash string `json:"executableHash"`
}

// decodeReceipt decodes the receipt of an installation and checks it.
func decodeReceipt(data []byte) (archiveReceipt, error) {
	var receipt archiveReceipt
	if err := json.Unmarshal(data, &receipt, wireOptions); err != nil {
		return archiveReceipt{}, wire.Refusal(err)
	}
	return receipt, receipt.validate()
}

// encodeReceipt returns the document of a receipt.
func encodeReceipt(receipt archiveReceipt) ([]byte, error) {
	if err := receipt.validate(); err != nil {
		return nil, err
	}
	return json.Marshal(receipt, wireOptions)
}
