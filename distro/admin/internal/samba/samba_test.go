// SPDX-License-Identifier: MPL-2.0

package samba

import (
	"flag"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var update = flag.Bool("update", false, "rewrite the image's default kinolune.conf")

// imageConf is the rendered default that ships in the image.
const imageConf = "../../../rootfs/etc/samba/smb.conf.d/kinolune.conf"

func TestImageDefaultIsRendered(t *testing.T) {
	want := Render(Settings{})
	if *update {
		if err := os.WriteFile(imageConf, []byte(want), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	got, err := os.ReadFile(imageConf)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != want {
		t.Fatalf("%s is not Render(defaults); run go test ./internal/samba -update", imageConf)
	}
}

func lines(s string) map[string]bool {
	m := map[string]bool{}
	for _, l := range strings.Split(s, "\n") {
		m[strings.TrimSpace(l)] = true
	}
	return m
}

func TestDefaultsNoGuest(t *testing.T) {
	conf := Render(Settings{})
	l := lines(conf)
	for _, want := range []string{
		"map to guest = Never", "restrict anonymous = 2", "[Media]", "[Users]", "[TimeMachine]",
		"guest ok = no", "valid users = @alpha-users @wheel @alpha-admin", "hide unreadable = yes",
		"fruit:time machine = yes", "path = /var/srv/timemachine/%U", "force group = alpha-users",
	} {
		if !l[want] {
			t.Errorf("missing %q", want)
		}
	}
	for _, bad := range []string{"guest ok = yes", "map to guest = Bad User", "read list = nobody"} {
		if l[bad] {
			t.Errorf("unexpected %q", bad)
		}
	}
	shares := Shares(Settings{})
	if len(shares) != 3 || shares[0].Guest {
		t.Errorf("%+v", shares)
	}
}

func TestGuestReadOnlyMedia(t *testing.T) {
	conf := Render(Settings{MediaGuestRead: true, TimeMachineOff: true})
	media, rest, _ := strings.Cut(strings.SplitN(conf, "[Media]", 2)[1], "[Users]")
	ml := lines(media)
	for _, want := range []string{"guest ok = yes", "read list = nobody", "valid users = @alpha-users @wheel @alpha-admin nobody"} {
		if !ml[want] {
			t.Errorf("Media: missing %q", want)
		}
	}
	// The guest never reaches the personal folders.
	if strings.Contains(rest, "guest ok = yes") || strings.Contains(rest, "nobody") {
		t.Errorf("guest outside Media:\n%s", rest)
	}
	if strings.Contains(conf, "[TimeMachine]") {
		t.Error("TimeMachine share rendered while off")
	}
	gl := lines(strings.SplitN(conf, "[Media]", 2)[0])
	if !gl["map to guest = Bad User"] || !gl["restrict anonymous = 0"] {
		t.Errorf("global guest policy:\n%s", conf)
	}
}

func TestSettingsRoundTrip(t *testing.T) {
	p := filepath.Join(t.TempDir(), "x", "settings.json")
	if s, err := LoadSettings(p); err != nil || s != (Settings{}) {
		t.Fatalf("missing file: %+v %v", s, err)
	}
	want := Settings{MediaGuestRead: true, TimeMachineOff: true}
	if err := SaveSettings(p, want); err != nil {
		t.Fatal(err)
	}
	if got, err := LoadSettings(p); err != nil || got != want {
		t.Fatalf("%+v %v", got, err)
	}
	if err := os.WriteFile(p, []byte("{"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadSettings(p); err == nil {
		t.Fatal("broken file accepted")
	}
}
