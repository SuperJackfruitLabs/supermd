//! Theme generator: converts the vendored base16 schemes in
//! `assets/base16/` into `assets/themes/*.toml`.
//! Run: `cargo run --example import_base16`.
//!
//! The output is committed, the same arrangement `build_docs` uses for
//! `site/docs/`: nothing converts at startup, the app just loads the
//! `.toml` files. A test in `src/base16.rs` regenerates every theme and
//! fails if a committed file has drifted from its source, so a stale
//! commit cannot pass CI.
//!
//! The mapping itself is not here. It lives in `src/base16.rs` -- pure,
//! tested inline, and measured against the four themes SuperMD wrote by
//! hand before the converter existed. This file only reads and writes.

#[path = "../src/base16.rs"]
mod base16;

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources = root.join("assets/base16");
    let out = root.join("assets/themes");
    let mut written = 0;
    for slug in base16::CURATED {
        let path = sources.join(format!("{slug}.yaml"));
        let yaml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let scheme = base16::Scheme::parse(slug, &yaml)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
        let toml = base16::render_toml(&scheme);
        let target = out.join(format!("{slug}.toml"));
        let unchanged = std::fs::read_to_string(&target).map(|old| old == toml).unwrap_or(false);
        if unchanged {
            println!("  unchanged  {slug}.toml");
        } else {
            std::fs::write(&target, &toml)
                .unwrap_or_else(|e| panic!("writing {}: {e}", target.display()));
            println!("  wrote      {slug}.toml  ({})", scheme.name);
            written += 1;
        }
    }
    println!(
        "{} theme(s) written, {} already current.",
        written,
        base16::CURATED.len() - written
    );
}
