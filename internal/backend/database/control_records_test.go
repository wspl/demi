package database

import (
	"errors"
	"testing"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

func TestProviderCredentialVersionsAndCascades(t *testing.T) {
	c, _ := testControl(t)
	ctx := t.Context()
	owner := testMaster(t, c)
	provider := NewProvider{
		ID:     "entry-1",
		Owner:  owner.ID,
		Family: "anthropic",
		Kind:   webapi.CredentialKindSubscription,
		Label:  "Claude",
	}
	account := CredentialWrite{ID: "account-1", Label: "one", Source: "login", Secret: []byte{1, 2, 3}}
	inserted, err := c.InsertProvider(ctx, provider, []CredentialWrite{account})
	require(t, err)
	if inserted == nil {
		t.Fatal("provider missing")
	}
	provider.ID = "entry-2"
	duplicate, err := c.InsertProvider(ctx, provider, nil)
	require(t, err)
	equal(t, (*ProviderRow)(nil), duplicate)
	read, err := c.Provider(ctx, inserted.ID)
	require(t, err)
	equal(t, &account.ID, read.Active)
	credential, err := c.Credential(ctx, inserted.ID, account.ID)
	require(t, err)
	equal(t, uint64(1), credential.Version)
	equal(t, account.Secret, credential.Secret)
	changed, err := c.ReplaceCredentialSecret(ctx, inserted.ID, account.ID, []byte{4}, 1)
	require(t, err)
	equal(t, true, changed)
	changed, err = c.ReplaceCredentialSecret(ctx, inserted.ID, account.ID, []byte{5}, 1)
	require(t, err)
	equal(t, false, changed)
	require(t, c.RemoveCredential(ctx, inserted.ID, account.ID))
	read, err = c.Provider(ctx, inserted.ID)
	require(t, err)
	equal(t, (*webapi.CredentialID)(nil), read.Active)
	changed, err = c.WriteCredential(ctx, inserted.ID, account)
	require(t, err)
	equal(t, true, changed)
	require(t, c.DeleteProvider(ctx, inserted.ID))
	credential, err = c.Credential(ctx, inserted.ID, account.ID)
	require(t, err)
	equal(t, (*CredentialRow)(nil), credential)
}

func TestWorkspaceForkAndCloudRecords(t *testing.T) {
	c, _ := testControl(t)
	ctx := t.Context()
	owner := testMaster(t, c)
	source := createConversation(t, c, owner.ID, 1)
	cloud, err := c.ManagedDeviceOrCreate(ctx, owner.ID)
	require(t, err)
	same, err := c.ManagedDeviceOrCreate(ctx, owner.ID)
	require(t, err)
	equal(t, cloud, same)
	require(t, c.RotateDeviceToken(ctx, cloud.ID, HashToken("boot")))
	read, err := c.DeviceByToken(ctx, HashToken("boot"))
	require(t, err)
	equal(t, &cloud, read)
	device, err := c.CreateDevice(ctx, owner.ID, "laptop", runnerwire.RunnerPlatformLinux, HashToken("paired"))
	require(t, err)
	workspace, err := c.CreateWorkspace(ctx, NewWorkspaceID(), owner.ID, device.ID, "/work", "Work")
	require(t, err)
	if workspace == nil {
		t.Fatal("workspace missing")
	}
	target := &webapi.ConversationTargetWorkspace{WorkspaceID: workspace.ID}
	changed, err := c.SwitchConversationTarget(
		ctx,
		source.ID,
		source.Target,
		target,
		TargetSwitch{
			From: &ExecutionCloud{Path: "/work"},
			To:   &ExecutionWorkspace{WorkspaceID: workspace.ID, DeviceID: device.ID, Path: "/work"},
		},
		SwitchEnds{},
	)
	require(t, err)
	equal(t, true, changed)
	err = c.DeleteWorkspace(ctx, owner.ID, workspace.ID)
	var inUse *WorkspaceInUseError
	if !errors.As(err, &inUse) || inUse.Count != 1 {
		t.Fatalf("deletion: %v", err)
	}
	operation := ForkOperation{
		ID:     conversation(2),
		Owner:  owner.ID,
		Source: source.ID,
		Block:  "a1",
		Metadata: ForkMetadata{
			Title:         "Fork",
			Target:        target,
			CreatedAt:     core.UnixEpoch,
			AttachedHosts: []AttachedHostRecord{},
		},
	}
	reserved, err := c.ReserveFork(ctx, operation)
	require(t, err)
	equal(t, &operation, reserved)
	again, err := c.ReserveFork(ctx, operation)
	require(t, err)
	equal(t, reserved, again)
	pending, err := c.PendingForks(ctx)
	require(t, err)
	equal(t, []ForkOperation{operation}, pending)
	published, err := c.PublishFork(ctx, operation.ID)
	require(t, err)
	equal(t, "Fork", published.Title)
	equal(t, target, published.Target)
	pending, err = c.PendingForks(ctx)
	require(t, err)
	equal(t, 0, len(pending))
	require(t, c.AnnounceCloudReset(ctx, owner.ID, "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a04"))
	require(t, c.AnnounceCloudReset(ctx, owner.ID, "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a04"))
	reset, err := c.AnnouncedCloudReset(ctx, source.ID)
	require(t, err)
	equal(t, new(webapi.OperationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a04")), reset)
	record, err := c.Conversation(ctx, source.ID)
	require(t, err)
	equal(t, uint64(2), record.ContextVersion)
	uses, err := c.CloudUses(ctx, owner.ID, &cloud.ID)
	require(t, err)
	equal(t, 2, len(uses))
	equal(t, false, uses[0].OnCloud)
}

func TestEmailChallengeCooldownAttemptsAndConsumption(t *testing.T) {
	c, clock := testControl(t)
	ctx := t.Context()
	owner := testMaster(t, c)
	account, err := c.Account(ctx, owner.ID)
	require(t, err)
	policy := ChallengePolicy{Lifetime: time.Hour, Cooldown: time.Minute, Attempts: 2}
	code := HashCode([]byte("key"), "challenge", "123456")
	issue := ChallengeIssue{
		User:         owner.ID,
		ID:           "challenge",
		Email:        "changed@example.test",
		PasswordHash: account.PasswordHash,
		CodeHash:     code,
	}
	expires, err := c.IssueEmailChallenge(ctx, issue, policy)
	require(t, err)
	_, err = c.IssueEmailChallenge(ctx, issue, policy)
	if !errors.Is(err, ErrCoolingDown) {
		t.Fatalf("cooldown: %v", err)
	}
	_, err = c.ConfirmEmailChallenge(
		ctx,
		owner.ID,
		issue.ID,
		HashCode([]byte("key"), issue.ID, "wrong"),
		policy.Attempts,
	)
	if !errors.Is(err, ErrInvalidCode) {
		t.Fatalf("wrong code: %v", err)
	}
	changed, err := c.ConfirmEmailChallenge(ctx, owner.ID, issue.ID, code, policy.Attempts)
	require(t, err)
	expected := owner
	expected.Email = issue.Email
	equal(t, expected, changed)
	_, err = c.ConfirmEmailChallenge(ctx, owner.ID, issue.ID, code, policy.Attempts)
	if !errors.Is(err, ErrInvalidCode) {
		t.Fatalf("reused code: %v", err)
	}
	clock.at = expires
}

func TestPasswordHashPHCSyntax(t *testing.T) {
	for _, text := range []string{
		"$argon2id",
		"$argon2id$v=19",
		"$argon2id$m=",
		"$argon2id$m=1,m=2",
		"$argon2id$v=19,m=1",
		"$argon2id$c2FsdA$AAAAAAAAAAAAAA",
	} {
		hash, err := ParsePasswordHash(text)
		require(t, err)
		equal(t, text, hash.Text())
	}
	for _, text := range []string{
		"argon2id",
		"$ARGON2",
		"$argon2id$v=019",
		"$argon2id$a=b=c",
		"$argon2id$abc",
		"$argon2id$c2FsdA$AAAA\nAAAAAAAAAA",
	} {
		if _, err := ParsePasswordHash(text); err == nil {
			t.Errorf("accepted invalid PHC %q", text)
		}
	}
}
