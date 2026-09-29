package artifact

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

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
	return decode[archiveReceipt](data)
}

// encodeReceipt returns the document of a receipt.
func encodeReceipt(receipt archiveReceipt) ([]byte, error) {
	return encode(receipt)
}
