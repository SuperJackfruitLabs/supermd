//! Regenerate site/og.png from site/og.svg:
//! `cargo run --example render_og`
fn main() {
    let svg = std::fs::read_to_string("site/og.svg").expect("read site/og.svg");
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_system_fonts();
    let mut opts = resvg::usvg::Options::default();
    opts.fontdb = std::sync::Arc::new(db);
    let tree = resvg::usvg::Tree::from_str(&svg, &opts).expect("parse svg");
    let size = tree.size();
    let (w, h) = (size.width().ceil() as u32, size.height().ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).expect("pixmap");
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    std::fs::write("site/og.png", pixmap.encode_png().expect("png")).expect("write");
    println!("wrote site/og.png ({w}x{h})");
}
