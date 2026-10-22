//! Command output as every `demi` command prints it (`crates/shared-types/src/output.rs`): tables aligned by terminal width, missing values and
//! control characters.

use demi_shared_types::output::{Fields, Table};

#[test]
fn a_table_aligns_its_columns_by_terminal_width_and_marks_missing_values() {
    let mut table = Table::new(["tab", "title", "created by", "url"]);
    table.row(["t100", "大模型接口", "agent 545", "https://example.com/a"]);
    table.row(["t137", "Docs", "", "https://example.com/b"]);
    table.row([Some("t2"), None, None, Some("https://example.com/c")]);
    // Each CJK character takes two terminal cells, so the column after the
    // title starts at the same cell on every row.
    assert_eq!(
        table.render(),
        "TAB   TITLE       CREATED BY  URL\n\
         t100  大模型接口  agent 545   https://example.com/a\n\
         t137  Docs        -           https://example.com/b\n\
         t2    -           -           https://example.com/c\n"
    );
}

/// A value cannot shift a column or command the terminal: a tab, a carriage
/// return, an escape sequence and a C1 control print as visible escapes,
/// measured as they print.
#[test]
fn a_control_character_prints_as_a_visible_escape() {
    let mut table = Table::new(["id", "title"]);
    table.row(["t1", "\x1b[2Jcleared\tand\rback\u{9b}"]);
    table.row(["t2", "plain"]);
    assert_eq!(
        table.render(),
        "ID  TITLE\n\
         t1  \\x1b[2Jcleared\\tand\\rback\\u{9b}\n\
         t2  plain\n"
    );
    let mut fields = Fields::new();
    fields.field("Title", "\x1b[31mred").text("Result", "one\ttwo\r\nthree\n");
    assert_eq!(fields.render(), "Title: \\x1b[31mred\nResult:\n  one\\ttwo\n  three\n");
}
