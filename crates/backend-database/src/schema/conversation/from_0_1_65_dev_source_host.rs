//! From the form an upload's source was kept in under the word *host*: a
//! device is called a device.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONVERSATION_FROM_SOURCE_HOST);

const CONVERSATION_FROM_SOURCE_HOST: &str = "ALTER TABLE attachments RENAME COLUMN source_host TO source_device;";
