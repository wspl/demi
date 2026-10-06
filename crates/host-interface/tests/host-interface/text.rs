//! The text a handler prints: sizes as the page writes them, so a size a
//! command prints reads the same as the page's.

use demi_host_interface::text::byte_size;

#[test]
fn a_size_reads_as_the_page_writes_it() {
    for (bytes, text) in [
        (12, "12 B"),
        (1536, "1.5 KB"),
        (412_000, "402 KB"),
        (8_600_000, "8.2 MB"),
        (20 * 1024 * 1024, "20 MB"),
    ] {
        assert_eq!(byte_size(bytes), text, "{bytes}");
    }
}
