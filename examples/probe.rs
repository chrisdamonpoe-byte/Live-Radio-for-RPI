// Diagnostic helper:
//   cargo run --example probe -- <hls-master-or-shoutcast-url>
#[path = "../src/api.rs"]
mod api;

fn main() {
    let url = std::env::args().nth(1).unwrap_or_default();
    if url.is_empty() {
        eprintln!("usage: probe <hls-master-or-shoutcast-url>");
        std::process::exit(2);
    }
    let found = if url.contains(".m3u8") {
        api::probe_hls(&url)
    } else {
        api::probe_icy(&url)
    };
    match found {
        Some(title) => println!("title: {title}"),
        None => println!("no title found"),
    }
}
