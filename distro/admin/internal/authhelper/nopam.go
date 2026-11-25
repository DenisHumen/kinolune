// SPDX-License-Identifier: MPL-2.0

//go:build !pam

package authhelper

// Available reports whether this binary was built with PAM.
func Available() bool { return false }

// NewSystem returns an authenticator that refuses everyone: this binary was
// built without `-tags pam` (developer builds, CI vet/test).
func NewSystem() Authenticator { return refuseAll{} }

type refuseAll struct{}

func (refuseAll) Authenticate(string, string) (Identity, error) { return Identity{}, ErrDenied }
