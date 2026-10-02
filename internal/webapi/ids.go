package webapi

// A user account's id, which the backend assigns: like every identity, a
// nonempty string compared exactly.
// +demi:id
// +demi:length chars min=1
type UserID string

// A provider entry's id, which the backend assigns.
// +demi:id
// +demi:length chars min=1
type ProviderID string

// A subscription account's id within its entry, such as
// `cred-3f2a9c01d4e5b6a7`.
// +demi:id
// +demi:length chars min=1
type CredentialID string

// A device login in progress, which the backend names when it starts.
// +demi:id
// +demi:length chars min=1
type LoginID string

// A device's id, which the backend assigns when the device is paired or
// its Cloud is first used.
// +demi:id
// +demi:length chars min=1
type DeviceID string

// A workspace's id, which the backend assigns.
// +demi:id
// +demi:length chars min=1
type WorkspaceID string

// An upload's id, which the backend assigns and a frame names the
// upload by.
// +demi:id
// +demi:length chars min=1
type AttachmentID string

// A conversation's id, which the web app chooses: a UUID, kept in the
// case it arrived in. No two conversations have ids that differ only in
// case (`storage.md` § Encodings and digests).
// +demi:id
// +demi:pattern ^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$
type ConversationID string

// A Cloud reset's id, which the page chooses: a UUID. A retry with the
// same id is the same reset (`managed-hosts.md` § System reset).
// +demi:id
// +demi:pattern ^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$
type OperationID string

// An expose's id, which the backend draws: a DNS label, and the only
// credential of the expose's URL.
// +demi:root
// +demi:id
// +demi:pattern ^[a-z2-7]{26}$
type ExposeID string
