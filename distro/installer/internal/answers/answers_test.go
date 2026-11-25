// SPDX-License-Identifier: MPL-2.0

package answers

import (
	"os"
	"strings"
	"testing"
)

const hash = "$y$j9T$7nVyZ3WBHdkfQ8WaD9Xgd.$Dn3Cl9mI6/ybf8z7FNnUW2ePrCVc6kB8BDu7Rbq3wf4"

func load(t *testing.T, js string) (*Answers, error) {
	t.Helper()
	return Load(strings.NewReader(js))
}

func TestMinimalGetsDefaults(t *testing.T) {
	a, err := load(t, `{"version":1,"disk":{"path":"/dev/nvme0n1"},"user":{"name":"denis","password":"secret"}}`)
	if err != nil {
		t.Fatal(err)
	}
	if a.Hostname != "kinolune" || a.Language != "ru" || a.Network.Mode != "dhcp" {
		t.Errorf("defaults: %+v", a)
	}
	if a.Image.Target != "ghcr.io/denishumen/alpha-os:stable" {
		t.Errorf("target %q", a.Image.Target)
	}
	if a.Image.Source != "containers-storage:ghcr.io/denishumen/alpha-os:stable" || a.Image.SourceRef() != "ghcr.io/denishumen/alpha-os:stable" {
		t.Errorf("source %q", a.Image.Source)
	}
	if !a.Image.Enforce() || a.Image.Runner != RunnerPodman {
		t.Errorf("image %+v", a.Image)
	}
	if a.Disk.DataPartition || a.Disk.RootSizeGiB != 0 {
		t.Errorf("disk %+v", a.Disk)
	}
}

func TestBetaChannelAndDataDefaults(t *testing.T) {
	a, err := load(t, `{"version":1,"disk":{"path":"/dev/sda","data_partition":true},
		"user":{"name":"admin","password_hash":"`+hash+`"},"image":{"channel":"beta"}}`)
	if err != nil {
		t.Fatal(err)
	}
	if a.Image.Target != "ghcr.io/denishumen/alpha-os:beta" || a.Disk.RootSizeGiB != 64 || a.Disk.DataPool != "media" {
		t.Errorf("%+v %+v", a.Image, a.Disk)
	}
}

func TestExampleFile(t *testing.T) {
	f, err := os.Open("../../examples/answers.json")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	if _, err := Load(f); err != nil {
		t.Fatal(err)
	}
}

func TestRejects(t *testing.T) {
	base := `"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"admin","password":"x"}`
	cases := map[string]struct{ js, want string }{
		"unknown field":   {`{` + base + `,"hostnme":"x"}`, `unknown field "hostnme"`},
		"version":         {`{"version":2,"disk":{"path":"/dev/sda"},"user":{"name":"a","password":"x"}}`, "version: must be 1"},
		"no disk":         {`{"version":1,"user":{"name":"admin","password":"x"}}`, "disk.path: required"},
		"disk not dev":    {`{"version":1,"disk":{"path":"sda"},"user":{"name":"admin","password":"x"}}`, "under /dev"},
		"root w/o data":   {`{"version":1,"disk":{"path":"/dev/sda","root_size_gib":30},"user":{"name":"admin","password":"x"}}`, "need disk.data_partition"},
		"small root":      {`{"version":1,"disk":{"path":"/dev/sda","data_partition":true,"root_size_gib":8},"user":{"name":"admin","password":"x"}}`, "at least 16 GiB"},
		"bad hostname":    {`{` + base + `,"hostname":"My_NAS"}`, "not a valid host name"},
		"reserved user":   {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"root","password":"x"}}`, "reserved"},
		"bad user":        {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"Denis","password":"x"}}`, "user.name"},
		"no password":     {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"admin"}}`, "password or password_hash is required"},
		"both passwords":  {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"admin","password":"x","password_hash":"` + hash + `"}}`, "not both"},
		"sha512 hash":     {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"admin","password_hash":"$6$abc$def"}}`, "yescrypt"},
		"bad ssh key":     {`{"version":1,"disk":{"path":"/dev/sda"},"user":{"name":"admin","password":"x","ssh_authorized_keys":["hello"]}}`, "ssh_authorized_keys[0]"},
		"local target":    {`{` + base + `,"image":{"target":"containers-storage:localhost/alpha-os:dev"}}`, "never receive updates"},
		"oci target":      {`{` + base + `,"image":{"target":"oci-archive:/run/payload.tar"}}`, "never receive updates"},
		"no registry":     {`{` + base + `,"image":{"target":"localhost/alpha-os:dev"}}`, "registry host"},
		"no tag":          {`{` + base + `,"image":{"target":"ghcr.io/denishumen/alpha-os"}}`, "tag or digest"},
		"source registry": {`{` + base + `,"image":{"source":"docker://ghcr.io/denishumen/alpha-os:stable"}}`, "containers-storage"},
		"runner":          {`{` + base + `,"image":{"runner":"docker"}}`, "image.runner"},
		"channel":         {`{` + base + `,"image":{"channel":"nightly"}}`, "image.channel"},
		"static no gw":    {`{` + base + `,"network":{"mode":"static","address":"192.168.1.10/24","dns":["1.1.1.1"]}}`, "gateway: required"},
		"gw outside":      {`{` + base + `,"network":{"mode":"static","address":"192.168.1.10/24","gateway":"10.0.0.1","dns":["1.1.1.1"]}}`, "outside 192.168.1.0/24"},
		"no prefix":       {`{` + base + `,"network":{"mode":"static","address":"192.168.1.10","gateway":"192.168.1.1","dns":["1.1.1.1"]}}`, "network.address"},
		"dhcp with addr":  {`{` + base + `,"network":{"address":"192.168.1.10/24"}}`, "only for mode"},
		"bad mode":        {`{` + base + `,"network":{"mode":"pppoe"}}`, "network.mode"},
		"karg space":      {`{` + base + `,"kernel_args":["quiet splash"]}`, "kernel_args[0]"},
		"language":        {`{` + base + `,"language":"Russian"}`, "language"},
		"trailing":        {`{` + base + `}{}`, "trailing data"},
	}
	for name, c := range cases {
		t.Run(name, func(t *testing.T) {
			_, err := load(t, c.js)
			if err == nil || !strings.Contains(err.Error(), c.want) {
				t.Fatalf("got %v, want error containing %q", err, c.want)
			}
		})
	}
}

func TestStaticNetworkAndLocalRegistryTarget(t *testing.T) {
	// The QEMU test (distro/tests/vm-install.sh) installs an unsigned image
	// from a registry on the VM host: a registry with a port is fine.
	a, err := load(t, `{"version":1,"disk":{"path":"/dev/disk/by-id/nvme-QEMU_NVMe_Ctrl_alpha0"},
		"user":{"name":"admin","password":"x"},
		"network":{"mode":"static","interface":"enp1s0","address":"192.168.1.10/24","gateway":"192.168.1.1","dns":["192.168.1.1","1.1.1.1"]},
		"image":{"source":"containers-storage:10.0.2.2:5055/alpha-os:dev","target":"10.0.2.2:5055/alpha-os:dev","enforce_signature_policy":false,"runner":"host"}}`)
	if err != nil {
		t.Fatal(err)
	}
	if a.Image.Enforce() {
		t.Error("enforce_signature_policy=false ignored")
	}
}

func TestAllErrorsAtOnce(t *testing.T) {
	_, err := load(t, `{"version":1,"disk":{"path":"x"},"user":{"name":"Root"},"hostname":"-bad"}`)
	if err == nil {
		t.Fatal("no error")
	}
	for _, want := range []string{"disk.path", "user.name", "password", "hostname"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("missing %q in %v", want, err)
		}
	}
}
