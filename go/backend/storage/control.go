package storage

import (
	"context"
	"database/sql"
	"errors"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type Control struct {
	db    *sql.DB
	clock core.Clock
}

func OpenControl(ctx context.Context, path string, clock core.Clock) (*Control, error) {
	db, err := openDatabase(ctx, path, controlV1)
	if err != nil {
		return nil, err
	}
	return &Control{db, clock}, nil
}
func (c *Control) Close() error { return sqliteError(c.db.Close()) }

type Account struct {
	User     webapi.UserDTO
	Password PasswordHash
}

const userColumns = "id, email, nickname, role, created_at"

func scanUser(row scanner, withHash bool) (*Account, error) {
	var id, email, nickname, role, hash string
	var ms int64
	dest := []any{&id, &email, &nickname, &role, &ms}
	if withHash {
		dest = append(dest, &hash)
	}
	if err := row.Scan(dest...); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, sqliteError(err)
	}
	uid, err := webapi.ParseUserID(id)
	if err != nil {
		return nil, corrupt("users", "id", err)
	}
	address, err := webapi.ParseEmailAddress(email)
	if err != nil {
		return nil, corrupt("users", "email", err)
	}
	if role != "master" && role != "admin" && role != "user" {
		return nil, &CorruptError{"users", "role", "unknown role: " + role}
	}
	created, err := instant("users", "created_at", ms)
	if err != nil {
		return nil, err
	}
	account := &Account{User: webapi.UserDTO{ID: uid, Email: address, Nickname: nickname, Role: webapi.Role(role), CreatedAt: created}}
	if withHash {
		account.Password, err = ParsePasswordHash(hash)
		if err != nil {
			return nil, corrupt("users", "password_hash", err)
		}
	}
	return account, nil
}
func (c *Control) HasUsers(ctx context.Context) (bool, error) {
	var exists bool
	err := c.db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users)").Scan(&exists)
	return exists, sqliteError(err)
}
func (c *Control) CreateMaster(ctx context.Context, email webapi.EmailAddress, hash PasswordHash) (*webapi.UserDTO, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	var exists bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users)").Scan(&exists); err != nil {
		return nil, sqliteError(err)
	}
	if exists {
		return nil, nil
	}
	user, err := insertUser(ctx, tx, c.clock.Now(), email, hash, webapi.RoleMaster)
	if err != nil {
		return nil, err
	}
	return user, sqliteError(tx.Commit())
}
func insertUser(ctx context.Context, db database, now core.Timestamp, email webapi.EmailAddress, hash PasswordHash, role webapi.Role) (*webapi.UserDTO, error) {
	account, err := scanUser(db.QueryRowContext(ctx, "INSERT INTO users (id,email,nickname,password_hash,role,created_at) VALUES (?,?,'',?,?,?) ON CONFLICT(email) DO NOTHING RETURNING "+userColumns, uuid.NewString(), email.String(), hash.PHC(), string(role), now.Millisecond()), false)
	if err != nil || account == nil {
		return nil, err
	}
	return &account.User, nil
}
func (c *Control) CreateUser(ctx context.Context, email webapi.EmailAddress, hash PasswordHash, role webapi.Role) (*webapi.UserDTO, error) {
	return insertUser(ctx, c.db, c.clock.Now(), email, hash, role)
}
func (c *Control) Account(ctx context.Context, id webapi.UserID) (*Account, error) {
	return scanUser(c.db.QueryRowContext(ctx, "SELECT "+userColumns+",password_hash FROM users WHERE id=?", id.String()), true)
}
func (c *Control) AccountByEmail(ctx context.Context, email webapi.EmailAddress) (*Account, error) {
	return scanUser(c.db.QueryRowContext(ctx, "SELECT "+userColumns+",password_hash FROM users WHERE email=?", email.String()), true)
}
func (c *Control) Users(ctx context.Context) ([]webapi.UserDTO, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+userColumns+" FROM users ORDER BY created_at,rowid")
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	users := []webapi.UserDTO{}
	for rows.Next() {
		account, err := scanUser(rows, false)
		if err != nil {
			return nil, err
		}
		users = append(users, account.User)
	}
	return users, sqliteError(rows.Err())
}
func (c *Control) EmailInUse(ctx context.Context, email webapi.EmailAddress) (bool, error) {
	var exists bool
	err := c.db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users WHERE email=?)", email.String()).Scan(&exists)
	return exists, sqliteError(err)
}
func (c *Control) SetNickname(ctx context.Context, id webapi.UserID, nickname string) (*webapi.UserDTO, error) {
	a, err := scanUser(c.db.QueryRowContext(ctx, "UPDATE users SET nickname=? WHERE id=? RETURNING "+userColumns, nickname, id.String()), false)
	if a == nil || err != nil {
		return nil, err
	}
	return &a.User, nil
}
func (c *Control) SetPassword(ctx context.Context, id webapi.UserID, hash PasswordHash) error {
	_, err := c.db.ExecContext(ctx, "UPDATE users SET password_hash=? WHERE id=?", hash.PHC(), id.String())
	return sqliteError(err)
}

type SessionPolicy struct{ Lifetime, RenewBelow time.Duration }
type ResolvedSession struct {
	User      webapi.UserDTO
	ExpiresAt core.Timestamp
	Renewed   bool
}

func (c *Control) OpenWebSession(ctx context.Context, hash TokenHash, user webapi.UserID, policy SessionPolicy) (core.Timestamp, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return core.Timestamp{}, sqliteError(err)
	}
	defer tx.Rollback()
	now := c.clock.Now()
	expiry, err := after(now, policy.Lifetime)
	if err != nil {
		return expiry, err
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM web_sessions WHERE expires_at<=?", now.Millisecond()); err != nil {
		return expiry, sqliteError(err)
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO web_sessions (token_hash,user_id,expires_at) VALUES (?,?,?)", hash.Text(), user.String(), expiry.Millisecond()); err != nil {
		return expiry, sqliteError(err)
	}
	return expiry, sqliteError(tx.Commit())
}
func (c *Control) ResolveWebSession(ctx context.Context, hash TokenHash, policy SessionPolicy) (*ResolvedSession, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	var ms int64
	var uid string
	err = tx.QueryRowContext(ctx, "SELECT user_id,expires_at FROM web_sessions WHERE token_hash=?", hash.Text()).Scan(&uid, &ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	expiry, err := instant("web_sessions", "expires_at", ms)
	if err != nil {
		return nil, err
	}
	now := c.clock.Now()
	if ms <= now.Millisecond() {
		if _, err = tx.ExecContext(ctx, "DELETE FROM web_sessions WHERE token_hash=?", hash.Text()); err != nil {
			return nil, sqliteError(err)
		}
		return nil, sqliteError(tx.Commit())
	}
	a, err := scanUser(tx.QueryRowContext(ctx, "SELECT "+userColumns+" FROM users WHERE id=?", uid), false)
	if err != nil || a == nil {
		return nil, err
	}
	renewed := ms-now.Millisecond() < policy.RenewBelow.Milliseconds()
	if renewed {
		expiry, err = after(now, policy.Lifetime)
		if err != nil {
			return nil, err
		}
		if _, err = tx.ExecContext(ctx, "UPDATE web_sessions SET expires_at=? WHERE token_hash=?", expiry.Millisecond(), hash.Text()); err != nil {
			return nil, sqliteError(err)
		}
	}
	return &ResolvedSession{a.User, expiry, renewed}, sqliteError(tx.Commit())
}
func (c *Control) CloseWebSession(ctx context.Context, hash TokenHash) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM web_sessions WHERE token_hash=?", hash.Text())
	return sqliteError(err)
}

type ChallengePolicy struct{ Lifetime, Cooldown time.Duration }
type ChallengeIssue struct {
	User     webapi.UserID
	ID       string
	Email    webapi.EmailAddress
	Password PasswordHash
	Code     CodeHash
}
type ChallengeOutcome struct {
	Kind ChallengeResult
	User *webapi.UserDTO
}
type ChallengeResult string

const (
	ChallengeChanged     ChallengeResult = "changed"
	ChallengeInvalidCode ChallengeResult = "invalid_code"
	ChallengeEmailTaken  ChallengeResult = "email_taken"
)

func (c *Control) IssueEmailChallenge(ctx context.Context, issue ChallengeIssue, policy ChallengePolicy) (*core.Timestamp, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	now := c.clock.Now()
	var sent int64
	err = tx.QueryRowContext(ctx, "SELECT sent_at FROM email_challenges WHERE user_id=?", issue.User.String()).Scan(&sent)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return nil, sqliteError(err)
	}
	if err == nil {
		if _, err = instant("email_challenges", "sent_at", sent); err != nil {
			return nil, err
		}
		if now.Millisecond()-sent < policy.Cooldown.Milliseconds() {
			return nil, nil
		}
	}
	expiry, err := after(now, policy.Lifetime)
	if err != nil {
		return nil, err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO email_challenges (user_id,id,email,password_hash,code_hash,expires_at,sent_at,attempts) VALUES (?,?,?,?,?,?,?,0)
 ON CONFLICT(user_id) DO UPDATE SET id=excluded.id,email=excluded.email,password_hash=excluded.password_hash,code_hash=excluded.code_hash,expires_at=excluded.expires_at,sent_at=excluded.sent_at,attempts=0`, issue.User.String(), issue.ID, issue.Email.String(), issue.Password.PHC(), issue.Code.text, expiry.Millisecond(), now.Millisecond())
	if err != nil {
		return nil, sqliteError(err)
	}
	return &expiry, sqliteError(tx.Commit())
}
func (c *Control) DeleteEmailChallenge(ctx context.Context, user webapi.UserID, id string) error {
	_, err := c.db.ExecContext(ctx, "DELETE FROM email_challenges WHERE user_id=? AND id=?", user.String(), id)
	return sqliteError(err)
}
func (c *Control) ConfirmEmailChallenge(ctx context.Context, user webapi.UserID, id string, code CodeHash, maxAttempts uint32) (ChallengeOutcome, error) {
	invalid := ChallengeOutcome{Kind: ChallengeInvalidCode}
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return invalid, sqliteError(err)
	}
	defer tx.Rollback()
	var email, password, hash string
	var expiry int64
	var attempts uint32
	err = tx.QueryRowContext(ctx, "SELECT email,password_hash,code_hash,expires_at,attempts FROM email_challenges WHERE user_id=? AND id=?", user.String(), id).Scan(&email, &password, &hash, &expiry, &attempts)
	if errors.Is(err, sql.ErrNoRows) {
		return invalid, nil
	}
	if err != nil {
		return invalid, sqliteError(err)
	}
	address, err := webapi.ParseEmailAddress(email)
	if err != nil {
		return invalid, corrupt("email_challenges", "email", err)
	}
	if _, err = instant("email_challenges", "expires_at", expiry); err != nil {
		return invalid, err
	}
	var current string
	err = tx.QueryRowContext(ctx, "SELECT password_hash FROM users WHERE id=?", user.String()).Scan(&current)
	if errors.Is(err, sql.ErrNoRows) {
		return invalid, nil
	}
	if err != nil {
		return invalid, sqliteError(err)
	}
	if expiry <= c.clock.Now().Millisecond() || attempts >= maxAttempts || current != password {
		return invalid, nil
	}
	if !code.matches(hash) {
		if _, err = tx.ExecContext(ctx, "UPDATE email_challenges SET attempts=attempts+1 WHERE user_id=?", user.String()); err != nil {
			return invalid, sqliteError(err)
		}
		return invalid, sqliteError(tx.Commit())
	}
	var taken bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users WHERE email=? AND id!=?)", address.String(), user.String()).Scan(&taken); err != nil {
		return invalid, sqliteError(err)
	}
	if taken {
		return ChallengeOutcome{Kind: ChallengeEmailTaken}, nil
	}
	a, err := scanUser(tx.QueryRowContext(ctx, "UPDATE users SET email=? WHERE id=? RETURNING "+userColumns, address.String(), user.String()), false)
	if err != nil || a == nil {
		return invalid, err
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM email_challenges WHERE user_id=?", user.String()); err != nil {
		return invalid, sqliteError(err)
	}
	return ChallengeOutcome{Kind: ChallengeChanged, User: &a.User}, sqliteError(tx.Commit())
}
