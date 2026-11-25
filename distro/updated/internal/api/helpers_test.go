// SPDX-License-Identifier: MPL-2.0

package api

import "runtime"

func isLinux() bool { return runtime.GOOS == "linux" }
