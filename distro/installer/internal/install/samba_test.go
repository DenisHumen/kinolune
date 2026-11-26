// SPDX-License-Identifier: MPL-2.0

package install

import (
	"encoding/hex"
	"testing"
)

func TestMD4Vectors(t *testing.T) {
	// RFC 1320 appendix A.5.
	for in, want := range map[string]string{
		"":    "31d6cfe0d16ae931b73c59d7e0c089c0",
		"a":   "bde52cb31de33e46245e05fbdbd6fb24",
		"abc": "a448017aaf21d8525fc10ae87aa6729d",
		"12345678901234567890123456789012345678901234567890123456789012345678901234567890": "e33b4ddc9c38f2199c3e7b164fcc0536",
	} {
		sum := md4([]byte(in))
		if got := hex.EncodeToString(sum[:]); got != want {
			t.Errorf("md4(%q) = %s, want %s", in, got, want)
		}
	}
}

// TestNTHash checks the well-known NT hash of the word "password" (a public
// test vector, not a secret) through the smbpasswd line Samba imports.
func TestNTHash(t *testing.T) {
	if got := SambaImportLine("anna", 1000, "password", 0x6a000000); got !=
		"anna:1000:XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX:8846F7EAEE8FB117AD06BDD830B7586C:[U          ]:LCT-6A000000:\n" {
		t.Fatalf("%q", got)
	}
}
