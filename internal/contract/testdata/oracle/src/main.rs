fn main() {
    if std::env::args().any(|arg| arg == "--numbers") {
        for input in [
            "1.0",
            "-0",
            "-0.0",
            "0.00001",
            "0.000001",
            "1000000000000000.0",
            "1e16",
            "1e20",
            "1e-20",
            "5e-324",
            "1.7976931348623157e308",
            "-9223372036854775808",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775809",
        ] {
            let value: serde_json::Value = serde_json::from_str(input).unwrap();
            println!(
                "{}",
                serde_json::json!({"input": input, "encoded": serde_json::to_string(&value).unwrap()})
            );
        }
        return;
    }

    let cases = [
        ("html", "<>&".to_owned()),
        ("separators", "\u{2028}\u{2029}".to_owned()),
        ("quotes", "\"\\/".to_owned()),
        ("controls", (0u8..32).map(char::from).collect()),
        ("unicode", "中文 café 😀\u{007f}".to_owned()),
        ("literal_escapes", "\\u2028\\u003c\\n".to_owned()),
    ];
    for (name, input) in cases {
        println!(
            "{}",
            serde_json::json!({"name": name, "input": input, "encoded": serde_json::to_string(&input).unwrap()})
        );
    }
    for depth in [126, 127, 128, 129] {
        let input = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        eprintln!(
            "{depth} containers: {}",
            serde_json::from_str::<serde_json::Value>(&input).is_ok()
        );
    }
}
