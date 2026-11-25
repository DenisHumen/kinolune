// SPDX-License-Identifier: MPL-2.0

package main

import (
	"net/http"
	"net/http/httptest"
	"slices"
	"testing"
)

func TestRedirect(t *testing.T) {
	cases := []struct{ addr, host, path, want string }{
		{":443", "kinolune.local", "/network?x=1", "https://kinolune.local/network?x=1"},
		{":443", "192.168.1.20:80", "/", "https://192.168.1.20/"},
		{":8443", "nas", "/a", "https://nas:8443/a"},
		{":443", "[fd00::5]:80", "/", "https://[fd00::5]/"},
	}
	for _, c := range cases {
		h := redirect(c.addr)
		r := httptest.NewRequest("GET", "http://x"+c.path, nil)
		r.Host = c.host
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		if w.Code != http.StatusMovedPermanently || w.Header().Get("Location") != c.want {
			t.Errorf("%+v: %d %q", c, w.Code, w.Header().Get("Location"))
		}
	}
	w := httptest.NewRecorder()
	redirect(":443").ServeHTTP(w, httptest.NewRequest("GET", "http://x/health", nil))
	if w.Code != 200 {
		t.Fatalf("/health on http: %d", w.Code)
	}
}

func TestSplitList(t *testing.T) {
	if got := splitList(" wheel, ,alpha-admin "); !slices.Equal(got, []string{"wheel", "alpha-admin"}) {
		t.Fatal(got)
	}
}
