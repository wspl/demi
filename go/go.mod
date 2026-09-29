module github.com/wspl/demi/go

go 1.27

require (
	github.com/aws/aws-sdk-go-v2 v1.47.1
	github.com/aws/aws-sdk-go-v2/config v1.33.6
	github.com/aws/aws-sdk-go-v2/credentials v1.20.6
	github.com/aws/aws-sdk-go-v2/service/s3 v1.113.4
	github.com/aws/smithy-go v1.28.1
	github.com/coder/websocket v1.8.15
	github.com/coreos/go-systemd/v22 v22.7.0
	github.com/google/uuid v1.6.0
	github.com/klauspost/compress v1.20.1
	github.com/modelcontextprotocol/go-sdk v1.8.0
	github.com/nlnwa/whatwg-url v0.6.2
	github.com/opencontainers/runtime-spec v1.3.0
	github.com/santhosh-tekuri/jsonschema/v6 v6.0.3
	github.com/tinylib/msgp v1.4.0
	github.com/vishvananda/netlink v1.3.1
	golang.org/x/crypto v0.57.0
	golang.org/x/crypto/x509roots/fallback v0.0.0-20260921070245-7a4a4d6beae2
	golang.org/x/sync v0.23.0
	golang.org/x/sys v0.48.0
	golang.org/x/text v0.42.0
	golang.org/x/tools v0.49.0
	modernc.org/sqlite v1.52.0
	mvdan.cc/sh/v3 v3.14.1
)

require (
	github.com/aws/aws-sdk-go-v2/aws/protocol/eventstream v1.7.20 // indirect
	github.com/aws/aws-sdk-go-v2/feature/ec2/imds v1.20.1 // indirect
	github.com/aws/aws-sdk-go-v2/internal/configsources v1.5.4 // indirect
	github.com/aws/aws-sdk-go-v2/internal/endpoints/v2 v2.8.4 // indirect
	github.com/aws/aws-sdk-go-v2/internal/v4a v1.5.4 // indirect
	github.com/aws/aws-sdk-go-v2/service/internal/accept-encoding v1.13.19 // indirect
	github.com/aws/aws-sdk-go-v2/service/internal/checksum v1.11.5 // indirect
	github.com/aws/aws-sdk-go-v2/service/internal/presigned-url v1.14.4 // indirect
	github.com/aws/aws-sdk-go-v2/service/internal/s3shared v1.20.4 // indirect
	github.com/aws/aws-sdk-go-v2/service/signin v1.10.1 // indirect
	github.com/aws/aws-sdk-go-v2/service/sso v1.38.1 // indirect
	github.com/aws/aws-sdk-go-v2/service/ssooidc v1.43.1 // indirect
	github.com/aws/aws-sdk-go-v2/service/sts v1.51.1 // indirect
	github.com/bits-and-blooms/bitset v1.20.0 // indirect
	github.com/dustin/go-humanize v1.0.1 // indirect
	github.com/google/jsonschema-go v0.4.3 // indirect
	github.com/mattn/go-isatty v0.0.24 // indirect
	github.com/ncruces/go-strftime v1.0.0 // indirect
	github.com/philhofer/fwd v1.2.0 // indirect
	github.com/remyoudompheng/bigfft v0.0.0-20230129092748-24d4a6f8daec // indirect
	github.com/segmentio/asm v1.1.3 // indirect
	github.com/segmentio/encoding v0.5.4 // indirect
	github.com/vishvananda/netns v0.0.5 // indirect
	github.com/yosida95/uritemplate/v3 v3.0.2 // indirect
	golang.org/x/mod v0.41.0 // indirect
	golang.org/x/net v0.59.0 // indirect
	golang.org/x/oauth2 v0.35.0 // indirect
	golang.org/x/term v0.46.0 // indirect
	golang.org/x/time v0.15.0 // indirect
	modernc.org/libc v1.72.5 // indirect
	modernc.org/mathutil v1.7.1 // indirect
	modernc.org/memory v1.12.1 // indirect
)

replace mvdan.cc/sh/v3 => ./third_party/mvdan-sh
