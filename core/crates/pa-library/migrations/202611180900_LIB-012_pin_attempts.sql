-- SPDX-License-Identifier: MPL-2.0
--
-- LIB-012 (hardening): the wrong-PIN backoff lives in the core, so no PIN
-- prompt of any client can skip it and a restart does not reset it.
-- Expand-only.
--
-- pin_attempts  one row (`one` = 1) for every PIN check of the library:
--               `failures` wrong PINs in a row since the last right one,
--               and until when no PIN is checked at all
--               (`locked_until_ms`, Unix ms; NULL = no wait). No row = no
--               wrong PINs. Rules: pa_library::profiles (5 free tries,
--               then 30 s doubling per wrong PIN up to 1 h).

CREATE TABLE pin_attempts (
    one             INTEGER PRIMARY KEY CHECK (one = 1),
    failures        INTEGER NOT NULL CHECK (failures >= 0),
    locked_until_ms INTEGER
) STRICT;
