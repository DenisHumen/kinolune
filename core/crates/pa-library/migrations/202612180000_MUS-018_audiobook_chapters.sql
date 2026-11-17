-- SPDX-License-Identifier: MPL-2.0
--
-- MUS-018 (ADR-NNNN): the audiobook library reads the chapters and the
-- audiobook tags (narrator, series, number in the series) of a file from
-- `files.embedded_tags_json`, which `pa-probe` fills from now on
-- (`"chapter_list"`, `"book"`). Files the classifier already called
-- audiobooks were probed without them: they are probed again on the next
-- scan (only the probe stamp is cleared; the new probe overwrites the other
-- probe columns). Data only, no schema change (LIB-002).

UPDATE files SET probed_at_ms = NULL
WHERE probed_at_ms IS NOT NULL
  AND id IN (SELECT file_id FROM file_content_kinds WHERE kind = 'audiobook');
