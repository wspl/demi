//! From 0.1.5979's control schema: a share can hold one reply block of a
//! conversation, by its tag and ID. A widget no longer opens in a tab of
//! the work panel: each panel's widget tabs go, their ids retired as a
//! closed tab's are, and the panel counts the change. A widget keeps its
//! page's title in place of a file's name made from it: the name without its
//! `.html`, which lost the title's punctuation, or none for the name of a
//! page without a title, `widget-w<n>.html` of its own number, and for a
//! name the plugin did not make.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979);

const CONTROL_FROM_0_1_5979: &str = "
CREATE TABLE shares_next (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer', 'reply_block')),
  block_id        TEXT,
  reply_block_tag TEXT,
  reply_block_id  TEXT,
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_tag IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_id IS NOT NULL)),
  CHECK (kind <> 'reply_block' OR title IS NOT NULL)
) STRICT;
INSERT INTO shares_next (token, user_id, conversation_id, kind, block_id, title, taken_at)
  SELECT token, user_id, conversation_id, kind, block_id, title, taken_at FROM shares;
DROP TABLE shares;
ALTER TABLE shares_next RENAME TO shares;
CREATE UNIQUE INDEX shares_of_conversation ON shares (conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (conversation_id, block_id) WHERE kind = 'answer';
CREATE UNIQUE INDEX shares_of_reply_block ON shares (conversation_id, reply_block_tag, reply_block_id)
  WHERE kind = 'reply_block';
UPDATE conversation_panels SET
  document = json_object(
    'tabs', (SELECT json_group_array(json(tab.value)) FROM json_each(document, '$.tabs') AS tab
             WHERE json_extract(tab.value, '$.kind') IS NOT 'widget'),
    'retired', (SELECT json_group_array(id) FROM (
                  SELECT retired.value AS id FROM json_each(document, '$.retired') AS retired
                  UNION ALL
                  SELECT json_extract(tab.value, '$.id') FROM json_each(document, '$.tabs') AS tab
                  WHERE json_extract(tab.value, '$.kind') = 'widget'))
  ),
  revision = revision + 1,
  updated_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
WHERE EXISTS (SELECT 1 FROM json_each(document, '$.tabs') AS tab WHERE json_extract(tab.value, '$.kind') = 'widget');
UPDATE plugin_values SET
  document = json_set(
    json_remove(document, '$.name'),
    '$.title',
    CASE WHEN json_extract(document, '$.name') = 'widget-w' || json_extract(document, '$.number') || '.html'
           OR json_extract(document, '$.name') NOT GLOB '?*.html' THEN json('null')
         ELSE substr(json_extract(document, '$.name'), 1, length(json_extract(document, '$.name')) - 5) END
  ),
  revision = revision + 1
WHERE plugin = 'widget' AND key GLOB 'w[0-9]*' AND json_type(document, '$.name') = 'text';
";
