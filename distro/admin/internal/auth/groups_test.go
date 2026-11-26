// SPDX-License-Identifier: MPL-2.0

package auth

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestGroupFiles(t *testing.T) {
	dir := t.TempDir()
	write := func(name, data string) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte(data), 0o644); err != nil {
			t.Fatal(err)
		}
		return p
	}
	f := GroupFiles{
		Passwd: []string{write("passwd", "anna:x:1000:1000::/home/anna:/bin/bash\nboris:x:1001:1001::/home/boris:/bin/bash\ncarla:x:1002:1002::/home/carla:/bin/bash\n"), filepath.Join(dir, "missing")},
		// /etc/group has wheel with anna; /usr/lib/group's wheel (with boris)
		// is shadowed, its alpha-admin (carla's primary group) is used.
		Group: []string{write("group", "wheel:x:10:anna\nanna:x:1000:\nboris:x:1001:\n"), write("lib-group", "wheel:x:10:boris\nalpha-admin:x:1002:\n")},
	}
	if g, ok := f.Groups("anna"); !ok || !slices.Equal(g, []string{"wheel", "anna"}) {
		t.Fatalf("anna: %v %v", g, ok)
	}
	if !f.IsAdminNow("anna", DefaultAdminGroups) || f.IsAdminNow("boris", DefaultAdminGroups) {
		t.Fatal("anna admin, boris not")
	}
	if !f.IsAdminNow("carla", DefaultAdminGroups) {
		t.Fatal("primary group alpha-admin")
	}
	if _, ok := f.Groups("deleted"); ok || f.IsAdminNow("deleted", DefaultAdminGroups) {
		t.Fatal("a missing user is no administrator")
	}
}

func TestRevokeUser(t *testing.T) {
	s := storeWith(newClock())
	a1, _ := s.Create("anna", 1000, RoleAdmin, "a")
	a2, _ := s.Create("anna", 1000, RoleAdmin, "b")
	b1, _ := s.Create("boris", 1001, RoleAdmin, "c")
	if n := s.RevokeUser("anna", a1); n != 1 {
		t.Fatalf("revoked %d", n)
	}
	if _, ok := s.Get(a1); !ok {
		t.Fatal("kept session gone")
	}
	if _, ok := s.Get(a2); ok {
		t.Fatal("other session of anna alive")
	}
	if n := s.RevokeUser("boris", ""); n != 1 {
		t.Fatal(n)
	}
	if _, ok := s.Get(b1); ok {
		t.Fatal("boris alive")
	}
}
