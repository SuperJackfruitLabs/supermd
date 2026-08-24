//! Docs-site generator: renders docs/site/*.md into site/docs/ with
//! the landing page's look. Run: `cargo run --example build_docs`.
//! Tests:   `cargo test --example build_docs`
//!
//! Content lives in docs/site/, ordered by docs/site/nav.toml; output
//! is committed (Cloudflare Pages serves static files, no CI build).

use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub file: String,
    pub title: String,
    pub group: String,
}

/// Parse nav.toml, preserving order. Errors on duplicate files.
fn parse_nav(src: &str) -> Result<Vec<Page>, String> {
    let value: toml::Value = toml::from_str(src).map_err(|e| e.to_string())?;
    let entries = value
        .get("pages")
        .and_then(|p| p.as_array())
        .ok_or("nav.toml needs [[pages]] entries")?;
    let mut pages = Vec::new();
    for entry in entries {
        let field = |k: &str| -> Result<String, String> {
            entry
                .get(k)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("nav entry missing `{k}`: {entry}"))
        };
        let page = Page { file: field("file")?, title: field("title")?, group: field("group")? };
        if pages.iter().any(|p: &Page| p.file == page.file) {
            return Err(format!("duplicate nav entry: {}", page.file));
        }
        pages.push(page);
    }
    Ok(pages)
}

/// `index.md` -> "", `editing.md` -> "editing".
fn slug_of(file: &str) -> String {
    let stem = file.strip_suffix(".md").unwrap_or(file);
    if stem == "index" { String::new() } else { stem.to_string() }
}

/// URL for a slug: "" -> "/docs/", "editing" -> "/docs/editing/".
fn url_of(slug: &str) -> String {
    if slug.is_empty() { "/docs/".to_string() } else { format!("/docs/{slug}/") }
}

/// Every nav entry must have a file; every md file must be in nav.
fn check_drift(nav: &[Page], files: &[String]) -> Result<(), String> {
    for page in nav {
        if !files.iter().any(|f| f == &page.file) {
            return Err(format!("nav lists {} but docs/site has no such file", page.file));
        }
    }
    for file in files {
        if !nav.iter().any(|p| &p.file == file) {
            return Err(format!("docs/site/{file} is not listed in nav.toml"));
        }
    }
    Ok(())
}

/// Rewrite internal `.md` hrefs to their /docs/ URLs (anchors kept).
/// Unknown internal `.md` targets are an error; external links pass.
fn rewrite_links(html: &str, slugs: &BTreeSet<String>) -> Result<String, String> {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(ix) = rest.find("href=\"") {
        let (head, tail) = rest.split_at(ix + "href=\"".len());
        out.push_str(head);
        let end = tail.find('"').ok_or("unterminated href")?;
        let target = &tail[..end];
        let rewritten = if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with('#')
            || target.starts_with('/')
        {
            target.to_string()
        } else if let Some((file, anchor)) = split_md_target(target) {
            let slug = slug_of(&file);
            if !slugs.contains(&slug) {
                return Err(format!("link to unknown page: {target}"));
            }
            match anchor {
                Some(a) => format!("{}#{a}", url_of(&slug)),
                None => url_of(&slug),
            }
        } else {
            target.to_string()
        };
        out.push_str(&rewritten);
        rest = &tail[end..];
    }
    out.push_str(rest);
    Ok(out)
}

/// "editing.md#tables" -> Some(("editing.md", Some("tables"))).
fn split_md_target(target: &str) -> Option<(String, Option<String>)> {
    let (file, anchor) = match target.split_once('#') {
        Some((f, a)) => (f, Some(a.to_string())),
        None => (target, None),
    };
    file.ends_with(".md").then(|| (file.to_string(), anchor))
}

fn md_options() -> pulldown_cmark::Options {
    pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS
}

/// Markdown -> HTML body (tables + strikethrough + tasklists on).
fn markdown_to_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(markdown, md_options());
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

/// First paragraph of the markdown as plain text (meta description).
fn first_paragraph_text(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut in_paragraph = false;
    let mut text = String::new();
    for event in Parser::new_ext(markdown, md_options()) {
        match event {
            Event::Start(Tag::Paragraph) => in_paragraph = true,
            Event::End(TagEnd::Paragraph) if in_paragraph => break,
            Event::Text(t) | Event::Code(t) if in_paragraph => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak if in_paragraph => text.push(' '),
            _ => {}
        }
    }
    text
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

// ── diagram previews ────────────────────────────────────────────────
// Fenced examples whose content is a ```mermaid or ```dot block get a
// real rendered preview injected below the code, in light and dark,
// using the same engines the app ships (merman; layout-rs like the
// bundled dot plugin).

struct DocsPalette {
    background: &'static str,
    surface: &'static str,
    text: &'static str,
    muted: &'static str,
    primary: &'static str,
    border: &'static str,
}

/// The landing page palette, mirrored for diagram theming.
fn docs_palette(dark: bool) -> DocsPalette {
    if dark {
        DocsPalette {
            background: "#2b2822",
            surface: "#262420",
            text: "#d9d4c8",
            muted: "#8f897a",
            primary: "#e5a63b",
            border: "#383428",
        }
    } else {
        DocsPalette {
            background: "#f6f2e9",
            surface: "#f8f5ec",
            text: "#33302a",
            muted: "#918b7d",
            primary: "#c9821c",
            border: "#eae5d8",
        }
    }
}

const DIAGRAM_FONT: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";

fn render_mermaid(source: &str, p: &DocsPalette) -> Result<String, String> {
    let site_config = serde_json::json!({
        "theme": "base",
        "htmlLabels": false,
        "flowchart": { "htmlLabels": false },
        "fontFamily": DIAGRAM_FONT,
        "themeVariables": {
            "background": p.background,
            "mainBkg": p.surface,
            "primaryColor": p.surface,
            "primaryTextColor": p.text,
            "primaryBorderColor": p.primary,
            "secondaryColor": p.surface,
            "secondaryTextColor": p.text,
            "tertiaryColor": p.background,
            "tertiaryTextColor": p.text,
            "lineColor": p.muted,
            "textColor": p.text,
            "nodeBorder": p.primary,
            "clusterBkg": p.background,
            "clusterBorder": p.border,
            "fontFamily": DIAGRAM_FONT,
        },
    });
    let pipeline = merman::svg::SvgOutputPolicy {
        preset: merman::svg::SvgPipelinePreset::ResvgSafe,
        css_override_policy: merman::svg::CssOverridePolicy::StripExistingImportant,
        root_background_color: Some(p.background.to_string()),
        drop_native_duplicate_fallbacks: false,
        scoped_css: None,
    }
    .pipeline();
    let renderer = merman::svg::HeadlessRenderer::new()
        .with_site_config(merman::MermaidConfig::from_value(site_config))
        .with_svg_pipeline(pipeline);
    match renderer.render_svg_sync(source) {
        Ok(Some(svg)) => Ok(svg),
        Ok(None) => Err("no mermaid diagram detected".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Same recolor approach as the bundled dot plugin.
fn render_dot(source: &str, p: &DocsPalette) -> Result<String, String> {
    use layout::backends::svg::SVGWriter;
    use layout::gv::{DotParser, GraphBuilder};
    let mut parser = DotParser::new(source);
    let graph = parser.process().map_err(|e| format!("dot parse: {e}"))?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    let mut writer = SVGWriter::new();
    visual.do_it(false, false, false, &mut writer);
    let svg = writer.finalize();
    Ok(svg
        .replace("fill=\"#ffffffff\"", &format!("fill=\"{}\"", p.surface))
        .replace("fill=\"#000000ff\"", &format!("fill=\"{}\"", p.text))
        .replace("stroke=\"#000000ff\"", &format!("stroke=\"{}\"", p.muted))
        .replace(
            "<svg ",
            &format!("<svg style=\"background-color:{}\" ", p.background),
        )
        .replace("<text ", &format!("<text fill=\"{}\" ", p.text))
        .replace("<text>", &format!("<text fill=\"{}\">", p.text)))
}

fn decode_entities(s: &str) -> String {
    s.replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Append rendered light+dark previews after every markdown example
/// whose content is a single ```mermaid or ```dot fence.
fn inject_diagram_previews(html: &str) -> Result<String, String> {
    const OPEN: &str = "<pre><code class=\"language-markdown\">```";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(ix) = rest.find(OPEN) {
        let after_open = ix + OPEN.len();
        let Some(end_rel) = rest[after_open..].find("</code></pre>") else {
            break;
        };
        let block_end = after_open + end_rel + "</code></pre>".len();
        out.push_str(&rest[..block_end]);
        let inner = &rest[after_open..after_open + end_rel];
        let (lang, body) = inner.split_once('\n').unwrap_or(("", ""));
        let source = decode_entities(body.trim_end_matches("```\n").trim_end_matches("```"));
        let rendered = match lang.trim() {
            "mermaid" => Some((
                render_mermaid(&source, &docs_palette(false))?,
                render_mermaid(&source, &docs_palette(true))?,
            )),
            "dot" | "graphviz" => Some((
                render_dot(&source, &docs_palette(false))?,
                render_dot(&source, &docs_palette(true))?,
            )),
            _ => None,
        };
        if let Some((light, dark)) = rendered {
            out.push_str(&format!(
                "\n<figure class=\"diagram-preview\">\
                 <div class=\"light\">{light}</div>\
                 <div class=\"dark\">{dark}</div>\
                 <figcaption>…renders as</figcaption></figure>\n"
            ));
        }
        rest = &rest[block_end..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The complete HTML document for one page.
fn render_page(page: &Page, nav: &[Page], markdown: &str) -> Result<String, String> {
    let ix = nav
        .iter()
        .position(|p| p.file == page.file)
        .ok_or_else(|| format!("{} not in nav", page.file))?;
    let slug = slug_of(&page.file);
    let body = inject_diagram_previews(&markdown_to_html(markdown))?;
    let description = escape_attr(&first_paragraph_text(markdown));

    let mut sidebar = String::new();
    let mut current_group = "";
    for p in nav {
        if p.group != current_group {
            if !current_group.is_empty() {
                sidebar.push_str("</ul>\n");
            }
            current_group = &p.group;
            sidebar.push_str(&format!("<h4>{}</h4>\n<ul>\n", p.group));
        }
        let class = if p.file == page.file { " class=\"current\"" } else { "" };
        sidebar.push_str(&format!(
            "<li{class}><a href=\"{}\">{}</a></li>\n",
            url_of(&slug_of(&p.file)),
            p.title
        ));
    }
    sidebar.push_str("</ul>\n");

    let mut pager = String::new();
    if ix > 0 {
        let prev = &nav[ix - 1];
        pager.push_str(&format!(
            "<a class=\"prev-link\" href=\"{}\">&larr; {}</a>",
            url_of(&slug_of(&prev.file)),
            prev.title
        ));
    }
    pager.push_str("<span class=\"pager-gap\"></span>");
    if ix + 1 < nav.len() {
        let next = &nav[ix + 1];
        pager.push_str(&format!(
            "<a class=\"next-link\" href=\"{}\">{} &rarr;</a>",
            url_of(&slug_of(&next.file)),
            next.title
        ));
    }

    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — SuperMD Docs</title>
<meta name="description" content="{description}">
<link rel="canonical" href="https://supermd.app{url}">
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'%3E%3Crect width='100' height='100' rx='22' fill='%23c9821c'/%3E%3Cpath d='M25 72V32l14 22 11-17 11 17 14-22v40' stroke='white' stroke-width='9' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E">
<style>
{css}
</style>
</head>
<body>
<header class="top">
  <a class="wordmark" href="/"><span class="badge"><svg viewBox="0 0 100 100"><path d="M25 72V32l14 22 11-17 11 17 14-22v40" stroke="white" stroke-width="9" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg></span><span class="name">Super<span class="mark">MD</span></span></a>
  <nav>
    <a href="/docs/">Docs</a>
    <a href="https://github.com/SuperJackfruitLabs/supermd/releases/latest">Download</a>
    <a href="https://github.com/SuperJackfruitLabs/supermd">GitHub</a>
  </nav>
</header>
<div class="layout">
<aside class="sidebar">
{sidebar}
</aside>
<main class="doc">
{body}
<div class="pager">{pager}</div>
</main>
</div>
<footer>© 2026 SuperJackfruitLabs · Apache-2.0</footer>
</body>
</html>
"#,
        title = page.title,
        description = description,
        url = url_of(&slug),
        css = PAGE_CSS,
        sidebar = sidebar,
        body = body,
        pager = pager,
    ))
}

/// Shared shell style — the landing page's palette, docs layout.
const PAGE_CSS: &str = r#"  :root {
    --bg: #fdfbf6; --fg: #33302a; --strong: #211f1a; --muted: #918b7d;
    --accent: #c9821c; --panel: #f8f5ec; --border: #eae5d8; --code: #f6f2e9;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #211f1a; --fg: #d9d4c8; --strong: #f2ede2; --muted: #8f897a;
      --accent: #e5a63b; --panel: #262420; --border: #383428; --code: #2b2822;
    }
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    background: var(--bg); color: var(--fg);
    font: 16px/1.65 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  a { color: var(--accent); text-decoration: none; }
  a:hover { text-decoration: underline; }
  header.top {
    display: flex; align-items: center; justify-content: space-between;
    padding: 18px 28px; border-bottom: 1px solid var(--border);
  }
  .wordmark {
    display: inline-flex; align-items: center; gap: 9px;
    font-weight: 800; font-size: 19px; color: var(--strong); letter-spacing: -0.015em;
    font-family: ui-rounded, "SF Pro Rounded", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  }
  .wordmark:hover { text-decoration: none; }
  .wordmark .badge {
    width: 26px; height: 26px; border-radius: 6px; flex: none;
    background: linear-gradient(180deg, #e5a63b, #c9821c);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .wordmark .badge svg { width: 17px; height: 17px; display: block; }
  .wordmark .mark { color: var(--accent); }
  header.top nav a { margin-left: 20px; color: var(--fg); font-size: 15px; }
  .layout { display: flex; max-width: 1080px; margin: 0 auto; }
  .sidebar {
    width: 220px; flex: none; padding: 32px 20px; font-size: 14px;
    border-right: 1px solid var(--border); min-height: calc(100vh - 61px);
  }
  .sidebar h4 {
    color: var(--muted); text-transform: uppercase; letter-spacing: 0.06em;
    font-size: 11px; margin: 18px 0 8px;
  }
  .sidebar h4:first-child { margin-top: 0; }
  .sidebar ul { list-style: none; }
  .sidebar li { margin: 2px 0; }
  .sidebar li a { display: block; color: var(--fg); padding: 4px 8px; border-radius: 6px; }
  .sidebar li a:hover { background: var(--panel); text-decoration: none; }
  .sidebar li.current a { background: var(--panel); color: var(--strong); font-weight: 600; }
  main.doc { flex: 1; min-width: 0; max-width: 44rem; padding: 36px 32px 64px; }
  .doc h1 { color: var(--strong); font-size: 32px; line-height: 1.2; margin-bottom: 18px; }
  .doc h2 { color: var(--strong); font-size: 22px; margin: 34px 0 10px; }
  .doc h3 { color: var(--strong); font-size: 17px; margin: 26px 0 8px; }
  .doc p, .doc ul, .doc ol { margin-bottom: 14px; }
  .doc ul, .doc ol { padding-left: 24px; }
  .doc li { margin: 4px 0; }
  .doc code {
    background: var(--code); border-radius: 4px; padding: 1px 5px; font-size: 13.5px;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
  }
  .doc pre {
    background: var(--code); border: 1px solid var(--border); border-radius: 8px;
    padding: 14px 16px; font-size: 13px; overflow-x: auto; margin-bottom: 14px;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
  }
  .doc pre code { background: none; padding: 0; font-size: 13px; }
  .doc table { border-collapse: collapse; margin-bottom: 14px; width: 100%; font-size: 14.5px; }
  .doc th, .doc td { border: 1px solid var(--border); padding: 6px 10px; text-align: left; }
  .doc th { background: var(--panel); color: var(--strong); }
  .doc blockquote {
    border-left: 3px solid var(--accent); padding-left: 14px; color: var(--muted);
    margin-bottom: 14px;
  }
  .pager {
    display: flex; justify-content: space-between; margin-top: 48px;
    padding-top: 18px; border-top: 1px solid var(--border); font-size: 15px;
  }
  footer {
    text-align: center; color: var(--muted); font-size: 13px;
    padding: 28px 0 40px; border-top: 1px solid var(--border);
  }
  .diagram-preview {
    background: var(--code); border: 1px solid var(--border); border-radius: 8px;
    padding: 18px; margin-bottom: 14px; text-align: center;
  }
  .diagram-preview svg { max-width: 100%; height: auto; }
  .diagram-preview .dark { display: none; }
  @media (prefers-color-scheme: dark) {
    .diagram-preview .light { display: none; }
    .diagram-preview .dark { display: block; }
  }
  .diagram-preview figcaption { color: var(--muted); font-size: 12px; margin-top: 8px; }
  @media (max-width: 900px) {
    .layout { flex-direction: column; }
    .sidebar { width: auto; border-right: none; border-bottom: 1px solid var(--border); min-height: 0; }
    main.doc { padding: 24px 20px 48px; }
  }
"#;

/// Replace the `<!-- docs -->`...`<!-- /docs -->` block with one <url>
/// per slug. Idempotent.
fn patch_sitemap(xml: &str, slugs: &[String]) -> Result<String, String> {
    const OPEN: &str = "<!-- docs -->";
    const CLOSE: &str = "<!-- /docs -->";
    let start = xml.find(OPEN).ok_or("sitemap.xml is missing the <!-- docs --> marker")?;
    let after = start + OPEN.len();
    let close_rel = xml[after..].find(CLOSE).ok_or("sitemap.xml is missing <!-- /docs -->")?;
    let close = after + close_rel;
    let mut block = String::from("\n");
    for slug in slugs {
        block.push_str(&format!(
            "<url><loc>https://supermd.app{}</loc></url>\n",
            url_of(slug)
        ));
    }
    Ok(format!("{}{}{}{}", &xml[..after], block, "", &xml[close..]))
}

/// Every internal /docs/ href in the generated pages must be a page.
fn internal_links_resolve(pages_html: &[(String, String)], slugs: &BTreeSet<String>) -> Result<(), String> {
    for (slug, html) in pages_html {
        let mut rest = html.as_str();
        while let Some(ix) = rest.find("href=\"/docs/") {
            let tail = &rest[ix + "href=\"".len()..];
            let end = tail.find('"').ok_or("unterminated href")?;
            let target = tail[..end].split('#').next().unwrap_or("");
            let target_slug = target
                .strip_prefix("/docs/")
                .map(|s| s.trim_end_matches('/'))
                .unwrap_or("");
            if !slugs.contains(target_slug) {
                return Err(format!("page {} links to unknown {target}", url_of(slug)));
            }
            rest = &tail[end..];
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = root.join("docs/site");
    let out_dir = root.join("site/docs");

    let nav = parse_nav(
        &std::fs::read_to_string(src_dir.join("nav.toml")).map_err(|e| e.to_string())?,
    )?;
    let files: Vec<String> = std::fs::read_dir(&src_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.ends_with(".md"))
        .collect();
    check_drift(&nav, &files)?;

    let slugs: BTreeSet<String> = nav.iter().map(|p| slug_of(&p.file)).collect();
    let mut rendered: Vec<(String, String)> = Vec::new(); // (slug, html)
    for page in &nav {
        let markdown = std::fs::read_to_string(src_dir.join(&page.file))
            .map_err(|e| format!("{}: {e}", page.file))?;
        let html = render_page(page, &nav, &markdown)?;
        let html = rewrite_links(&html, &slugs)?;
        rendered.push((slug_of(&page.file), html));
    }
    internal_links_resolve(&rendered, &slugs)?;

    // Clean output dir, then write pages at clean URLs.
    if out_dir.exists() {
        std::fs::remove_dir_all(&out_dir).map_err(|e| e.to_string())?;
    }
    for (slug, html) in &rendered {
        let dir = if slug.is_empty() { out_dir.clone() } else { out_dir.join(slug) };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("index.html"), html).map_err(|e| e.to_string())?;
        println!("wrote {}", url_of(slug));
    }

    let sitemap_path = root.join("site/sitemap.xml");
    let slug_list: Vec<String> = nav.iter().map(|p| slug_of(&p.file)).collect();
    let xml = std::fs::read_to_string(&sitemap_path).map_err(|e| e.to_string())?;
    std::fs::write(&sitemap_path, patch_sitemap(&xml, &slug_list)?)
        .map_err(|e| e.to_string())?;
    println!("patched sitemap.xml ({} docs urls)", slug_list.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAV: &str = r#"
[[pages]]
file = "index.md"
title = "Getting Started"
group = "Using SuperMD"

[[pages]]
file = "editing.md"
title = "Editing"
group = "Using SuperMD"

[[pages]]
file = "plugins.md"
title = "Plugins"
group = "Extending SuperMD"
"#;

    fn nav() -> Vec<Page> {
        parse_nav(NAV).unwrap()
    }

    fn slugs() -> BTreeSet<String> {
        nav().iter().map(|p| slug_of(&p.file)).collect()
    }

    #[test]
    fn nav_parses_in_order_and_rejects_duplicates() {
        let pages = nav();
        assert_eq!(
            pages.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            ["Getting Started", "Editing", "Plugins"]
        );
        assert_eq!(pages[2].group, "Extending SuperMD");
        let dup = format!("{NAV}\n[[pages]]\nfile = \"editing.md\"\ntitle = \"X\"\ngroup = \"G\"\n");
        assert!(parse_nav(&dup).is_err());
    }

    #[test]
    fn slugs_and_urls_map_cleanly() {
        assert_eq!(slug_of("index.md"), "");
        assert_eq!(slug_of("editing.md"), "editing");
        assert_eq!(url_of(""), "/docs/");
        assert_eq!(url_of("editing"), "/docs/editing/");
    }

    #[test]
    fn drift_is_caught_both_directions() {
        let files = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(check_drift(&nav(), &files(&["index.md", "editing.md", "plugins.md"])).is_ok());
        let missing = check_drift(&nav(), &files(&["index.md", "editing.md"]));
        assert!(missing.unwrap_err().contains("plugins.md"));
        let orphan = check_drift(
            &nav(),
            &files(&["index.md", "editing.md", "plugins.md", "stray.md"]),
        );
        assert!(orphan.unwrap_err().contains("stray.md"));
    }

    #[test]
    fn internal_links_rewrite_and_unknown_targets_error() {
        let html = r##"<a href="editing.md">e</a> <a href="index.md">i</a>
<a href="editing.md#tables">anchor</a>
<a href="https://example.com/x.md">ext</a> <a href="#local">l</a>"##;
        let out = rewrite_links(html, &slugs()).unwrap();
        assert!(out.contains(r#"href="/docs/editing/""#), "{out}");
        assert!(out.contains(r#"href="/docs/""#), "{out}");
        assert!(out.contains(r##"href="/docs/editing/#tables""##), "{out}");
        assert!(out.contains(r#"href="https://example.com/x.md""#), "external untouched");
        assert!(out.contains(r##"href="#local""##), "page anchor untouched");
        assert!(rewrite_links(r#"<a href="ghost.md">g</a>"#, &slugs()).is_err());
    }

    #[test]
    fn markdown_renders_tables_and_code() {
        let html = markdown_to_html("# H\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn x() {}\n```\n");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<pre><code"), "{html}");
    }

    #[test]
    fn first_paragraph_becomes_description() {
        let md = "# Title\n\nSuperMD is a **hybrid** editor.\nSecond line.\n\nNot this.\n";
        assert_eq!(
            first_paragraph_text(md),
            "SuperMD is a hybrid editor. Second line."
        );
    }

    #[test]
    fn rendered_page_carries_shell_and_nav() {
        let pages = nav();
        let html = render_page(&pages[1], &pages, "# Editing\n\nBody text.\n").unwrap();
        assert!(html.contains("<title>Editing — SuperMD Docs</title>"), "{html}");
        assert!(html.contains(r#"name="description" content="Body text.""#));
        assert!(html.contains(r#"rel="canonical" href="https://supermd.app/docs/editing/""#));
        for p in &pages {
            assert!(html.contains(&p.title), "sidebar lists {}", p.title);
        }
        assert!(html.contains(r#"class="current""#));
        assert!(html.contains("Using SuperMD") && html.contains("Extending SuperMD"));
        // prev/next from nav order
        assert!(html.contains(r#"href="/docs/""#), "prev link to index");
        assert!(html.contains(r#"href="/docs/plugins/""#), "next link");
        // first page has no prev, last no next
        let first = render_page(&pages[0], &pages, "# A\n\nx.\n").unwrap();
        assert!(!first.contains("prev-link"), "{first}");
        let last = render_page(&pages[2], &pages, "# Z\n\nx.\n").unwrap();
        assert!(!last.contains("next-link"), "{last}");
    }

    #[test]
    fn sitemap_patch_is_idempotent() {
        let xml = "<urlset>\n<url><loc>https://supermd.app/</loc></url>\n<!-- docs -->\nold\n<!-- /docs -->\n</urlset>";
        let slugs = vec!["".to_string(), "editing".to_string()];
        let once = patch_sitemap(xml, &slugs).unwrap();
        assert!(once.contains("<loc>https://supermd.app/docs/</loc>"), "{once}");
        assert!(once.contains("<loc>https://supermd.app/docs/editing/</loc>"));
        assert!(!once.contains("old"));
        let twice = patch_sitemap(&once, &slugs).unwrap();
        assert_eq!(once, twice, "idempotent");
        assert!(patch_sitemap("<urlset></urlset>", &slugs).is_err(), "missing markers");
    }

    #[test]
    fn diagram_examples_gain_rendered_previews() {
        let md = "````markdown\n```mermaid\nflowchart TD\n  A --> B\n```\n````\n\n\
                  ````markdown\n```dot\ndigraph { a -> b }\n```\n````\n\n\
                  ````markdown\n```rust\nfn x() {}\n```\n````\n";
        let html = inject_diagram_previews(&markdown_to_html(md)).unwrap();
        assert_eq!(html.matches("diagram-preview").count(), 2, "mermaid + dot only");
        assert_eq!(html.matches("class=\"light\"").count(), 2);
        assert_eq!(html.matches("class=\"dark\"").count(), 2);
        assert!(html.contains("<svg"), "real SVGs injected");
        // both palettes actually differ
        assert!(html.contains("#f6f2e9") && html.contains("#2b2822"), "light+dark rendered");
        // the original code samples stay visible above the previews
        assert!(html.contains("```mermaid"));
    }

    #[test]
    fn link_check_catches_dangling_docs_hrefs() {
        let ok = vec![("".to_string(), r#"<a href="/docs/editing/">e</a>"#.to_string())];
        assert!(internal_links_resolve(&ok, &slugs()).is_ok());
        let bad = vec![("".to_string(), r#"<a href="/docs/ghost/">g</a>"#.to_string())];
        assert!(internal_links_resolve(&bad, &slugs()).is_err());
    }
}
