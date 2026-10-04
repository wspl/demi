package blobs

import (
	"context"
	"fmt"
	"net/http"
	"os"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/config"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/credentials/ec2rolecreds"
	"github.com/aws/aws-sdk-go-v2/credentials/endpointcreds"
	"github.com/aws/aws-sdk-go-v2/credentials/stscreds"
	"github.com/aws/aws-sdk-go-v2/feature/ec2/imds"
	"github.com/aws/aws-sdk-go-v2/service/sts"
)

// storageCredentials selects only the object store's documented credential
// sources. LoadDefaultConfig cannot disable AWS_PROFILE (even with empty file
// lists), so selection is explicit; the SDK still owns credential protocols,
// refresh and caching. Every provider borrows the bucket's HTTP transport.
func storageCredentials(ctx context.Context, region string, client *http.Client) (aws.CredentialsProvider, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	env, err := config.NewEnvConfig()
	if err != nil {
		return nil, err
	}
	var provider aws.CredentialsProvider
	switch {
	case os.Getenv("AWS_ACCESS_KEY_ID") != "" || os.Getenv("AWS_SECRET_ACCESS_KEY") != "":
		if os.Getenv("AWS_ACCESS_KEY_ID") == "" {
			//nolint:staticcheck // ST1005: the diagnostic names the missing AWS field.
			return nil, fmt.Errorf(
				"Missing AccessKeyId",
			)
		}
		if os.Getenv("AWS_SECRET_ACCESS_KEY") == "" {
			//nolint:staticcheck // ST1005: the diagnostic names the missing AWS field.
			return nil, fmt.Errorf("Missing SecretAccessKey")
		}
		provider = credentials.StaticCredentialsProvider{Value: env.Credentials}
	case env.WebIdentityTokenFilePath != "" && env.RoleARN != "":
		service := sts.New(sts.Options{Region: region, HTTPClient: client})
		session := env.RoleSessionName
		if session == "" {
			session = "WebIdentitySession"
		}
		provider = stscreds.NewWebIdentityRoleProvider(
			service,
			env.RoleARN,
			stscreds.IdentityTokenFile(env.WebIdentityTokenFilePath),
			func(o *stscreds.WebIdentityRoleOptions) {
				o.RoleSessionName = session
			},
		)
	case env.ContainerCredentialsRelativePath != "" ||
		(env.ContainerCredentialsEndpoint != "" && os.Getenv("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE") != ""):
		endpoint := env.ContainerCredentialsEndpoint
		if env.ContainerCredentialsRelativePath != "" {
			endpoint = "http://169.254.170.2" + env.ContainerCredentialsRelativePath
		}
		provider = endpointcreds.New(endpoint, func(o *endpointcreds.Options) {
			o.HTTPClient = client
			if file := os.Getenv("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE"); file != "" {
				o.AuthorizationTokenProvider = endpointcreds.TokenProviderFunc(func() (string, error) {
					data, err := os.ReadFile(file)
					if err != nil {
						return "", fmt.Errorf("read container authorization token: %w", err)
					}
					return string(data), nil
				})
			}
		})
	default:
		metadata := imds.New(imds.Options{HTTPClient: client})
		provider = ec2rolecreds.New(func(o *ec2rolecreds.Options) {
			o.Client = metadata
		})
	}
	return provider, nil
}
