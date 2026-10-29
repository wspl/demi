//! The control database's history: each schema a server may hold before the
//! current one, oldest first, and the step from it to the next.
//!
//! The schemas of the subagent limits, without Detect Title, of the kept
//! limits and of the plugins' secrets and their read states no server keeps,
//! but development databases do, and a server's migrate through them.

use super::Shipped;

mod ledger;
mod shared;
mod from_0_1_11;
mod from_0_1_13;
mod from_0_1_15;
mod from_0_1_16;
mod from_0_1_17;
mod from_0_1_18;
mod from_0_1_20;
mod from_0_1_21;
mod from_0_1_22_dev_20261011t123543z;
mod from_0_1_22_dev_subagent_limits;
mod from_0_1_22_dev_no_detect_title;
mod from_0_1_22_dev_kept_limits;
mod from_0_1_22_dev_plugin_secrets_unread;
mod from_0_1_22_dev_plugin_secrets;
mod from_0_1_22_dev_credentials;
mod from_0_1_22_dev_passkeys;
mod from_0_1_22_dev_signed_in_browsers;
mod from_0_1_22_dev_saved_messages;
mod from_0_1_22_dev_administered_accounts;
mod from_0_1_22_dev_workflows;
mod from_0_1_22_dev_saved_edits;
mod from_0_1_22_dev_device_names;
mod from_0_1_22_dev_profile_contexts;
mod from_0_1_22_dev_config_format_2;
mod from_0_1_22_dev_account_rotation;
mod from_0_1_22_dev_placeholder_titles;
mod from_0_1_22_dev_interrupted_turns;
mod from_0_1_22_dev_reachable_hosts;
mod from_0_1_22_dev_shares;
mod from_0_1_22_dev_20261015t105826z;
mod from_0_1_22_dev_20261015t134426z;
mod from_0_1_22_dev_20261016t043704z;
mod from_0_1_22_dev_20261016t095626z;
mod from_0_1_22;
mod from_0_1_25;
mod from_0_1_27;
mod from_0_1_32;
mod from_0_1_36;
mod from_0_1_37;
mod from_0_1_44;
mod from_0_1_48;
mod from_0_1_63;
mod from_0_1_65;
mod from_0_1_5409;
mod from_0_1_5469;
mod from_0_1_5541;
mod from_0_1_5565;
mod from_0_1_5586;
mod from_0_1_5614;
mod from_0_1_5658;
mod from_0_1_5672;

pub(super) const HISTORY: &[Shipped] = &[
    Shipped {
        sql: include_str!("from_0_1_11.sql"),
        migration: from_0_1_11::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_13.sql"),
        migration: from_0_1_13::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_15.sql"),
        migration: from_0_1_15::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_16.sql"),
        migration: from_0_1_16::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_17.sql"),
        migration: from_0_1_17::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_18.sql"),
        migration: from_0_1_18::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_20.sql"),
        migration: from_0_1_20::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_21.sql"),
        migration: from_0_1_21::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_20261011t123543z.sql"),
        migration: from_0_1_22_dev_20261011t123543z::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_subagent_limits.sql"),
        migration: from_0_1_22_dev_subagent_limits::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_no_detect_title.sql"),
        migration: from_0_1_22_dev_no_detect_title::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_kept_limits.sql"),
        migration: from_0_1_22_dev_kept_limits::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_plugin_secrets_unread.sql"),
        migration: from_0_1_22_dev_plugin_secrets_unread::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_plugin_secrets.sql"),
        migration: from_0_1_22_dev_plugin_secrets::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_credentials.sql"),
        migration: from_0_1_22_dev_credentials::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_passkeys.sql"),
        migration: from_0_1_22_dev_passkeys::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_signed_in_browsers.sql"),
        migration: from_0_1_22_dev_signed_in_browsers::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_saved_messages.sql"),
        migration: from_0_1_22_dev_saved_messages::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_administered_accounts.sql"),
        migration: from_0_1_22_dev_administered_accounts::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_workflows.sql"),
        migration: from_0_1_22_dev_workflows::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_saved_edits.sql"),
        migration: from_0_1_22_dev_saved_edits::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_device_names.sql"),
        migration: from_0_1_22_dev_device_names::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_profile_contexts.sql"),
        migration: from_0_1_22_dev_profile_contexts::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_config_format_2.sql"),
        migration: from_0_1_22_dev_config_format_2::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_account_rotation.sql"),
        migration: from_0_1_22_dev_account_rotation::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_placeholder_titles.sql"),
        migration: from_0_1_22_dev_placeholder_titles::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_interrupted_turns.sql"),
        migration: from_0_1_22_dev_interrupted_turns::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_reachable_hosts.sql"),
        migration: from_0_1_22_dev_reachable_hosts::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_shares.sql"),
        migration: from_0_1_22_dev_shares::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_20261015t105826z.sql"),
        migration: from_0_1_22_dev_20261015t105826z::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_20261015t134426z.sql"),
        migration: from_0_1_22_dev_20261015t134426z::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_20261016t043704z.sql"),
        migration: from_0_1_22_dev_20261016t043704z::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22_dev_20261016t095626z.sql"),
        migration: from_0_1_22_dev_20261016t095626z::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_22.sql"),
        migration: from_0_1_22::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_25.sql"),
        migration: from_0_1_25::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_27.sql"),
        migration: from_0_1_27::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_32.sql"),
        migration: from_0_1_32::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_36.sql"),
        migration: from_0_1_36::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_37.sql"),
        migration: from_0_1_37::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_44.sql"),
        migration: from_0_1_44::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_48.sql"),
        migration: from_0_1_48::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_63.sql"),
        migration: from_0_1_63::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_65.sql"),
        migration: from_0_1_65::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5409.sql"),
        migration: from_0_1_5409::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5469.sql"),
        migration: from_0_1_5469::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5541.sql"),
        migration: from_0_1_5541::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5565.sql"),
        migration: from_0_1_5565::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5586.sql"),
        migration: from_0_1_5586::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5614.sql"),
        migration: from_0_1_5614::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5658.sql"),
        migration: from_0_1_5658::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5672.sql"),
        migration: from_0_1_5672::MIGRATION,
    },
];
