// SPDX-License-Identifier: MPL-2.0
//! Prints what NET-010 finds on this machine's LAN:
//!
//!   cargo run -p pa-lan-probe --example lan_search [wait-ms]

fn main() {
    let wait = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(2500);
    let opts = pa_lan_probe::Options {
        wait: std::time::Duration::from_millis(wait),
        ..pa_lan_probe::Options::default()
    };
    let started = std::time::Instant::now();
    match pa_lan_probe::discover(&opts) {
        Ok(found) => {
            for s in &found {
                println!(
                    "{:?}\t{}\t{}\t{}",
                    s.kind,
                    s.name,
                    s.url,
                    s.workgroup.as_deref().unwrap_or("-")
                );
            }
            println!("{} found in {:?}", found.len(), started.elapsed());
        }
        Err(e) => println!("search failed: {e}"),
    }
}
