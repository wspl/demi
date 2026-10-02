package plugin

import "github.com/wspl/demi/internal/contract"

// validatePageCall preserves the object-only page parameter contract.
func validatePageCall(request RequestPageCall) error {
	_, err := contract.ObjectFields(request.Params)
	return contract.At("params", err)
}

// validatePackageCall preserves the object-only package argument contract.
func validatePackageCall(message PortMessagePackageCall) error {
	_, err := contract.ObjectFields(message.Args)
	return contract.At("args", err)
}
