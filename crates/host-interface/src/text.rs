//! The text an `rpc` handler prints for a person or a model to read: rows in
//! aligned columns.

/// `rows` as lines of columns, each column as wide as its widest cell and
/// two spaces from the next; the last column is not padded.
pub fn table<const N: usize>(rows: &[[String; N]]) -> String {
    let mut widths = [0; N];
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut text = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(column, (cell, width))| {
                if column + 1 == N {
                    cell.clone()
                } else {
                    format!("{cell:<width$}")
                }
            })
            .collect();
        text.push_str(&cells.join("  "));
        text.push('\n');
    }
    text
}
