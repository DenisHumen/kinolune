#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
# NET-010 test share: nmbd (NetBIOS), smbd and wsdd (WS-Discovery) in
# one container.
set -e
nmbd -D
smbd -D
exec wsdd --interface eth0 --ipv4only --hostname KINO-WINPC --workgroup KINOTEST
