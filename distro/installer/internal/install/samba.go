// SPDX-License-Identifier: MPL-2.0

package install

import (
	binenc "encoding/binary"
	"encoding/hex"
	"fmt"
	"math/bits"
	"strconv"
	"strings"
	"unicode/utf16"
)

// SambaImportPath is where the installer leaves the first user's Samba
// password for alpha-samba-config.service (DIST-018): an smbpasswd-format
// line that `pdbedit -i` imports at the first boot, then deletes. Root only.
const SambaImportPath = "/var/lib/alpha/samba/import.smbpasswd"

// SambaImportLine is the smbpasswd line of user with password: no LM hash,
// the NT hash (MD4 of the UTF-16LE password), a normal account.
func SambaImportLine(user string, uid int, password string, now int64) string {
	return fmt.Sprintf("%s:%d:%s:%s:[U          ]:LCT-%08X:\n",
		user, uid, strings.Repeat("X", 32), strings.ToUpper(hex.EncodeToString(NTHash(password))), now)
}

// uidOf finds the UID of name in a passwd file.
func uidOf(passwd, name string) (int, error) {
	recs, err := readColon(passwd)
	if err != nil {
		return 0, err
	}
	for _, r := range recs {
		if len(r) > 2 && r[0] == name {
			return strconv.Atoi(r[2])
		}
	}
	return 0, fmt.Errorf("%s: no user %q", passwd, name)
}

// NTHash is the NT hash Samba stores for a password: MD4(UTF-16LE(pw)).
func NTHash(password string) []byte {
	u := utf16.Encode([]rune(password))
	b := make([]byte, 2*len(u))
	for i, c := range u {
		binenc.LittleEndian.PutUint16(b[2*i:], c)
	}
	sum := md4(b)
	return sum[:]
}

// md4 is RFC 1320 MD4, only for the NT hash above (the Go standard library
// has none; golang.org/x/crypto/md4 is deprecated).
func md4(msg []byte) [16]byte {
	a0, b0, c0, d0 := uint32(0x67452301), uint32(0xefcdab89), uint32(0x98badcfe), uint32(0x10325476)
	ml := uint64(len(msg)) * 8
	msg = append(append([]byte{}, msg...), 0x80)
	for len(msg)%64 != 56 {
		msg = append(msg, 0)
	}
	msg = binenc.LittleEndian.AppendUint64(msg, ml)
	f := func(x, y, z uint32) uint32 { return x&y | ^x&z }
	g := func(x, y, z uint32) uint32 { return x&y | x&z | y&z }
	h := func(x, y, z uint32) uint32 { return x ^ y ^ z }
	var x [16]uint32
	for off := 0; off < len(msg); off += 64 {
		for i := range x {
			x[i] = binenc.LittleEndian.Uint32(msg[off+4*i:])
		}
		a, b, c, d := a0, b0, c0, d0
		for _, i := range [4]int{0, 4, 8, 12} {
			a = bits.RotateLeft32(a+f(b, c, d)+x[i], 3)
			d = bits.RotateLeft32(d+f(a, b, c)+x[i+1], 7)
			c = bits.RotateLeft32(c+f(d, a, b)+x[i+2], 11)
			b = bits.RotateLeft32(b+f(c, d, a)+x[i+3], 19)
		}
		for _, i := range [4]int{0, 1, 2, 3} {
			a = bits.RotateLeft32(a+g(b, c, d)+x[i]+0x5a827999, 3)
			d = bits.RotateLeft32(d+g(a, b, c)+x[i+4]+0x5a827999, 5)
			c = bits.RotateLeft32(c+g(d, a, b)+x[i+8]+0x5a827999, 9)
			b = bits.RotateLeft32(b+g(c, d, a)+x[i+12]+0x5a827999, 13)
		}
		for _, i := range [4]int{0, 2, 1, 3} {
			a = bits.RotateLeft32(a+h(b, c, d)+x[i]+0x6ed9eba1, 3)
			d = bits.RotateLeft32(d+h(a, b, c)+x[i+8]+0x6ed9eba1, 9)
			c = bits.RotateLeft32(c+h(d, a, b)+x[i+4]+0x6ed9eba1, 11)
			b = bits.RotateLeft32(b+h(c, d, a)+x[i+12]+0x6ed9eba1, 15)
		}
		a0, b0, c0, d0 = a0+a, b0+b, c0+c, d0+d
	}
	var out [16]byte
	binenc.LittleEndian.PutUint32(out[0:], a0)
	binenc.LittleEndian.PutUint32(out[4:], b0)
	binenc.LittleEndian.PutUint32(out[8:], c0)
	binenc.LittleEndian.PutUint32(out[12:], d0)
	return out
}
