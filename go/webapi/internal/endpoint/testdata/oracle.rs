use std::io::{self, Read};
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&input).unwrap();
    let output: Vec<_> = rows.as_array().unwrap().iter().map(|row| {
        let text = row["input"].as_str().unwrap();
        let url = url::Url::parse(text).ok().filter(|url| matches!(url.scheme(), "http" | "https") && url.has_host()).map(|url| url.to_string());
        let absolute = typed_path::Utf8TypedPath::derive(text).is_absolute();
        serde_json::json!({"input":text,"endpoint":url,"absolute":absolute})
    }).collect();
    println!("{}",serde_json::to_string_pretty(&output).unwrap());
}
