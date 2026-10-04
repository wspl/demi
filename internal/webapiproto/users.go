package webapiproto

// `GET /users`: every account, in the order they were created.
// +demi:root direction=receive output=web
// +demi:tolerant
type Users struct {
	Users []UserDTO `json:"users"`
}

// The role an administrator gives a new account; only setup makes the
// master.
// +demi:enum admin user
type NewRole string

// Values of the preceding enumeration.
const (
	NewRoleAdmin NewRole = "admin"
	NewRoleUser  NewRole = "user"
)

// `POST /users`: a new account, which signs in without a verification
// email.
// +demi:root direction=send output=web
type CreateUser struct {
	// An `EmailAddress` is valid once it is decoded.
	Email EmailAddress `json:"email"`
	// +demi:length chars min=8 max=1024
	Password Password `json:"password"`
	Role     NewRole  `json:"role"`
}

// `{ user }`: the 201 answer of `POST /users`.
// +demi:root direction=receive output=web
// +demi:tolerant
type CreatedUser struct {
	User UserDTO `json:"user"`
}

// `PATCH /users/:id`: the new password of an account the caller outranks.
// +demi:root direction=send output=web
type PasswordReset struct {
	// +demi:length chars min=8 max=1024
	Password Password `json:"password"`
}

// Role is the existing account role corresponding to the new account role.
func (r NewRole) Role() (Role, error) {
	if err := r.Validate(); err != nil {
		return "", err
	}
	return Role(r), nil
}
