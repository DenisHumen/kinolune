// SPDX-License-Identifier: MPL-2.0

//go:build pam

package authhelper

import (
	"errors"
	"os/user"
	"strconv"

	"github.com/msteinert/pam/v2"
)

// PAMService is the PAM service name (/etc/pam.d/alpha-admin).
const PAMService = "alpha-admin"

// PAM checks passwords with PAM (auth + account) and resolves groups via NSS.
// Built only with `-tags pam` (needs cgo and the PAM headers); the image
// build passes it.
type PAM struct{ Service string }

// Available reports whether this binary was built with PAM.
func Available() bool { return true }

// NewSystem returns the PAM authenticator for the "alpha-admin" service.
func NewSystem() Authenticator { return PAM{Service: PAMService} }

// Authenticate implements Authenticator.
func (p PAM) Authenticate(name, password string) (Identity, error) {
	answered := false
	tx, err := pam.StartFunc(p.Service, name, func(s pam.Style, msg string) (string, error) {
		switch s {
		case pam.PromptEchoOff:
			// Only one password prompt is expected; a second one is a
			// multi-factor stack we cannot drive from a web form.
			if answered {
				return "", errors.New("unexpected second prompt")
			}
			answered = true
			return password, nil
		case pam.PromptEchoOn:
			return "", errors.New("unexpected prompt")
		default: // ErrorMsg, TextInfo
			return "", nil
		}
	})
	if err != nil {
		return Identity{}, ErrDenied
	}
	defer tx.End()
	if err := tx.Authenticate(pam.Silent | pam.DisallowNullAuthtok); err != nil {
		return Identity{}, ErrDenied
	}
	if err := tx.AcctMgmt(pam.Silent | pam.DisallowNullAuthtok); err != nil {
		return Identity{}, ErrDenied
	}
	return lookupIdentity(name)
}

func lookupIdentity(name string) (Identity, error) {
	u, err := user.Lookup(name)
	if err != nil {
		return Identity{}, ErrDenied
	}
	uid, err := strconv.Atoi(u.Uid)
	if err != nil {
		return Identity{}, ErrDenied
	}
	gids, err := u.GroupIds()
	if err != nil {
		return Identity{}, ErrDenied
	}
	groups := make([]string, 0, len(gids))
	for _, gid := range gids {
		if g, err := user.LookupGroupId(gid); err == nil {
			groups = append(groups, g.Name)
		}
	}
	return Identity{User: u.Username, UID: uid, Groups: groups}, nil
}
