// Package machinesproto is the contract between the backend and the Cloud
// machine manager (docs/cloud/managed-hosts.md): the messages of the manager's
// socket and their line codec, the record of a device's stored images and the
// Cloud image manifest. The manager and the backend both import it, so each is
// defined once.
//
// The types are declared for cmd/wiregen (go generate), which writes their
// decoders and rule checks: the requests are an adjacently tagged union of op and
// params flattened into the request, the messages ignore members they do not
// declare, and resetId is present and may be null. Every refusal names the field
// and the rule and never the value: a boot record holds a credential.
package machinesproto
