package webapi

import (
	"errors"
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// Where an expose's traffic goes on its device: `host:port` exactly as
// given, or a bare port, which means `127.0.0.1:<port>`. The host is any
// name or address the device can resolve, an IPv6 address in brackets; the
// port is 1 to 65535.
type ExposeAddress string

// ErrExposeAddress explains why a text is not an expose's address.
var ErrExposeAddress = errors.New("must be host:port or a port, the port 1 to 65535")

// ParseExposeAddress expands a bare port and validates a Host's address.
func ParseExposeAddress(text string) (ExposeAddress, error) {
	if port, err := exposePort(text); err == nil {
		return ExposeAddress("127.0.0.1:" + strconv.FormatUint(uint64(port), 10)), nil
	}
	if _, _, err := splitExposeAddress(text); err != nil {
		return "", err
	}
	return ExposeAddress(text), nil
}

// Host returns the host without IPv6 brackets, refusing an invalid value.
func (address ExposeAddress) Host() (string, error) {
	host, _, err := splitExposeAddress(string(address))
	return strings.TrimSuffix(strings.TrimPrefix(host, "["), "]"), err
}

// Port returns the port, refusing an invalid value.
func (address ExposeAddress) Port() (uint16, error) {
	_, port, err := splitExposeAddress(string(address))
	return port, err
}

func splitExposeAddress(address string) (string, uint16, error) {
	i := strings.LastIndexByte(address, ':')
	if i < 1 {
		return "", 0, ErrExposeAddress
	}
	host := address[:i]
	for _, b := range []byte(host) {
		if b < 0x21 || b > 0x7e || b == '/' {
			return "", 0, ErrExposeAddress
		}
	}
	bracketed := strings.HasPrefix(host, "[") || strings.HasSuffix(host, "]")
	if bracketed && (len(host) <= 2 || !strings.HasPrefix(host, "[") || !strings.HasSuffix(host, "]")) || !bracketed && strings.Contains(host, ":") {
		return "", 0, ErrExposeAddress
	}
	port, err := exposePort(address[i+1:])
	return host, port, err
}

func exposePort(text string) (uint16, error) {
	if text == "" {
		return 0, ErrExposeAddress
	}
	for _, b := range []byte(text) {
		if b < '0' || b > '9' {
			return 0, ErrExposeAddress
		}
	}
	port, err := strconv.ParseUint(text, 10, 16)
	if err != nil {
		return 0, fmt.Errorf("%w: %w", ErrExposeAddress, err)
	}
	if port == 0 {
		return 0, ErrExposeAddress
	}
	return uint16(port), nil
}

// +demi:root
type exposeAddressText string

// DecodeExposeAddress decodes and normalizes an expose address.
func DecodeExposeAddress(data []byte) (ExposeAddress, error) {
	text, err := DecodeexposeAddressText(data)
	if err != nil {
		return "", err
	}
	return ParseExposeAddress(string(text))
}

// UnmarshalJSON validates and normalizes a wire address.
func (address *ExposeAddress) UnmarshalJSON(data []byte) error {
	value, err := DecodeExposeAddress(data)
	if err != nil {
		return err
	}
	*address = value
	return nil
}

// MarshalJSON writes the normalized wire address.
func (address ExposeAddress) MarshalJSON() ([]byte, error) {
	value, err := ParseExposeAddress(string(address))
	if err != nil {
		return nil, err
	}
	return contract.EncodeJSON(string(value))
}
