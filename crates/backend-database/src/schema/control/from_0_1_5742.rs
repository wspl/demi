//! From 0.1.5742's control schema: an entry's switches of its vendor's own
//! web search and X search, and of their looking at images and videos, on.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(
    "ALTER TABLE providers ADD COLUMN web_search INTEGER NOT NULL DEFAULT 1 CHECK (web_search IN (0, 1));
     ALTER TABLE providers ADD COLUMN x_search INTEGER NOT NULL DEFAULT 1 CHECK (x_search IN (0, 1));
     ALTER TABLE providers ADD COLUMN search_images INTEGER NOT NULL DEFAULT 1 CHECK (search_images IN (0, 1));
     ALTER TABLE providers ADD COLUMN search_videos INTEGER NOT NULL DEFAULT 1 CHECK (search_videos IN (0, 1));",
);
