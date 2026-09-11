//! Scan a directory and print what was found. A developer's smoke test, not a product.
//!
//! `cargo run -p rl-discovery --example scan -- path/to/project`

use rl_model::ParamStyle;

fn main() {
    let root = std::env::args().nth(1).expect("usage: scan <project dir>");
    let started = std::time::Instant::now();
    let result = rl_discovery::scan(&root).unwrap_or_else(|e| panic!("scan failed: {e}"));
    let elapsed = started.elapsed();

    for f in &result.frameworks {
        println!(
            "framework {} (score {}): {}",
            f.id,
            f.score,
            f.evidence.join("; ")
        );
    }
    for b in &result.base_urls {
        println!("base {} <- {} ({})", b.url, b.source, b.confidence);
    }
    println!();
    for e in &result.endpoints {
        let flags = [
            e.orphaned.then_some("ORPHAN"),
            (!e.path.is_resolved()).then_some("UNRESOLVED"),
            e.auth.is_some().then_some("auth"),
            e.body.is_some().then_some("body"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        let source = e
            .source
            .as_ref()
            .map(|s| format!("{}:{}", s.file.display(), s.line))
            .unwrap_or_default();
        println!(
            "{:7} {:50} {:14} {:40} {}",
            e.method.as_str(),
            e.path.render(ParamStyle::Braces),
            e.group.as_deref().unwrap_or("-"),
            source,
            flags
        );
    }
    println!();
    for w in &result.warnings {
        println!("warning: {w}");
    }
    println!("{:?} in {:.0?}", result.stats, elapsed);
}
