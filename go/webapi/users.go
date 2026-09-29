package webapi

// `GET /users`: every account, in the order they were created.
//
//demi:wire open
type Users struct {
	Users []UserDTO `json:"users"`
}

// The role an administrator gives a new account; only setup makes the
// master.
//
//demi:enum
//demi:export
type NewRole string

const (
	NewRoleAdmin NewRole = "admin"
	NewRoleUser  NewRole = "user"
)

// `POST /users`: a new account, which signs in without a verification
// email.
//
//demi:wire
type CreateUser struct {
	// An `EmailAddress` is valid once it is decoded.
	Email    EmailAddress `json:"email" check:"func=Validate"`
	Password Password     `json:"password" check:"chars=8..1024,func=Validate"`
	Role     NewRole      `json:"role"`
}

// `{ user }`: the 201 answer of `POST /users`.
//
//demi:wire open
type CreatedUser struct {
	User UserDTO `json:"user"`
}

// `PATCH /users/:id`: the new password of an account the caller outranks.
//
//demi:wire
type PasswordReset struct {
	Password Password `json:"password" check:"chars=8..1024,func=Validate"`
}
