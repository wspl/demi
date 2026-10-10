//! Which blocks a page of a transcript holds (`web-api.md` § Pages): whole
//! requests within the page budget, from where the page is taken.

use demi_conversation_socket_protocol::{PAGE_BYTES, PageAt, page, requests};

// Cost: pure, under a millisecond.

#[test]
fn requests_start_at_user_blocks_and_the_blocks_before_the_first_join_it() {
    assert_eq!(requests(0, &[]), Vec::<std::ops::Range<usize>>::new());
    assert_eq!(requests(5, &[]), [0..5]);
    assert_eq!(requests(6, &[0, 2, 5]), [0..2, 2..5, 5..6]);
    // A subagent's brief may follow a context block.
    assert_eq!(requests(6, &[1, 4]), [0..4, 4..6]);
}

#[test]
fn a_page_holds_whole_requests_within_the_budget_and_at_least_one() {
    // Five requests of ten blocks each; `size` gives each request's bytes.
    let five = requests(50, &[0, 10, 20, 30, 40]);
    let third = PAGE_BYTES / 3;
    let even = |_: std::ops::Range<usize>| third;
    let cases: &[(&str, PageAt, &dyn Fn(std::ops::Range<usize>) -> usize, std::ops::Range<usize>)] = &[
        ("the latest takes requests back while they fit", PageAt::Latest, &even, 20..50),
        ("before an edge, the requests that end there", PageAt::Before(30), &even, 0..30),
        ("before an edge inside a request, that request whole", PageAt::Before(25), &even, 0..30),
        ("after an edge, the requests that start there", PageAt::After(9), &even, 10..40),
        ("around a block, its request, then after it, then before it", PageAt::Around(22), &even, 10..40),
        ("around the last block, the requests before it", PageAt::Around(49), &even, 20..50),
        ("a request past the budget alone", PageAt::Latest, &|_| PAGE_BYTES * 2, 40..50),
        ("nothing before the start", PageAt::Before(0), &even, 0..0),
        ("nothing after the end", PageAt::After(49), &even, 50..50),
    ];
    for (name, at, size, expected) in cases {
        assert_eq!(page(&five, *at, size), *expected, "{name}");
    }
    assert_eq!(page(&[], PageAt::Latest, |_| 0), 0..0, "an empty transcript");
}
