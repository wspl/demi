//! The text an `rpc` handler prints for a person or a model to read: rows in
//! aligned columns, and a byte count as a size.

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

/// `bytes` as a size, in the units and rounding the page writes sizes in
/// (`formatBytes` of `web-ui`): `12 B`, `1.5 KB`, `402 KB`, `20 MB`. A
/// kilobyte is 1,024 bytes, and a value under 10 keeps one decimal.
pub fn byte_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // A size is far below the 2^53 bytes up to which f64 counts exactly.
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{} {}", value.round(), UNITS[unit])
    }
}
