-- SPDX-License-Identifier: MPL-2.0
--
-- META-017: a user fix becomes a folder rule with release templates, and
-- rules travel between paired devices as `folder_rule.set` ops.
--
-- folder_rules.templates_json   release templates learned from the fixed
--                               names: `[{"pattern", "absolute", "season",
--                               "offset", "abs_offset"}]` (pa_catalog
--                               `folder_rules`); a new file named the same
--                               way takes its episode from its number
--                               without asking a provider
-- folder_rule_sync              what the family agrees on per rule key
--                               (`<source sync id>:<folder hash>`, never a
--                               path): `rule` the last sent or applied rule
--                               (NULL: deleted), `stamp` `{hlc, op_id}` of
--                               that op (last writer wins), `state`:
--                               `synced`, `received` (a rule of another
--                               device whose folder or source is not here
--                               yet; the next scan tries again) or
--                               `deleted` (removed here, the delete is
--                               still to send)
--
-- Expand only (LIB-002).

ALTER TABLE folder_rules ADD COLUMN templates_json TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(templates_json));

CREATE TABLE folder_rule_sync (
    rule_key TEXT    PRIMARY KEY CHECK (rule_key <> ''),
    rule     TEXT    CHECK (rule IS NULL OR json_valid(rule)),
    stamp    TEXT    CHECK (stamp IS NULL OR json_valid(stamp)),
    state    TEXT    NOT NULL DEFAULT 'synced'
                     CHECK (state IN ('synced', 'received', 'deleted'))
) STRICT;
