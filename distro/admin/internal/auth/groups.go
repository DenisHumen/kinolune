// SPDX-License-Identifier: MPL-2.0

package auth

import (
	"os"
	"slices"
	"strings"
)

// GroupFiles answers "is this user an administrator right now?" from the
// account files, for every request of a session (a user who was deleted or
// lost the rights must not keep using an old session). It reads the files
// itself instead of os/user: the daemon is static (no NSS), and on bootc
// systems groups like wheel may live in /usr/lib/group (nss-altfiles).
type GroupFiles struct {
	Passwd []string // /etc/passwd, /usr/lib/passwd
	Group  []string // /etc/group, /usr/lib/group
}

// DefaultGroupFiles are the files of the OS.
var DefaultGroupFiles = GroupFiles{
	Passwd: []string{"/etc/passwd", "/usr/lib/passwd"},
	Group:  []string{"/etc/group", "/usr/lib/group"},
}

func colonRecords(paths []string) [][]string {
	var out [][]string
	for _, p := range paths {
		b, err := os.ReadFile(p)
		if err != nil {
			continue
		}
		for _, l := range strings.Split(string(b), "\n") {
			if l != "" && !strings.HasPrefix(l, "#") {
				out = append(out, strings.Split(l, ":"))
			}
		}
	}
	return out
}

// Groups returns the names of the groups of user (primary and
// supplementary) and whether the user exists. The first record of a name
// wins, as with nss "files altfiles".
func (f GroupFiles) Groups(user string) ([]string, bool) {
	gid := ""
	for _, r := range colonRecords(f.Passwd) {
		if len(r) > 3 && r[0] == user {
			gid = r[3]
			break
		}
	}
	if gid == "" {
		return nil, false
	}
	seen := map[string]bool{}
	var groups []string
	for _, r := range colonRecords(f.Group) {
		if len(r) < 4 || seen[r[0]] {
			continue
		}
		seen[r[0]] = true
		if r[2] == gid || slices.Contains(strings.Split(r[3], ","), user) {
			groups = append(groups, r[0])
		}
	}
	return groups, true
}

// IsAdminNow reports whether user exists and is in one of adminGroups.
func (f GroupFiles) IsAdminNow(user string, adminGroups []string) bool {
	groups, ok := f.Groups(user)
	return ok && IsAdmin(groups, adminGroups)
}
