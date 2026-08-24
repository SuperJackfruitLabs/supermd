//! Test fixture: exercises the host-api fetch import (through
//! format-document) and export-document. The host test suite drives
//! the whole net-enforcement ladder through this plugin.

wit_bindgen::generate!({ path: "../../wit-v3", world: "extension" });

use supermd::extension::host_api;
use supermd::extension::types as t;

struct Plugin;

fn fetch_one(url: &str) -> Result<String, String> {
    let resp = host_api::fetch(&host_api::FetchRequest {
        method: "GET".into(),
        url: url.into(),
        headers: vec![],
        body: None,
    })?;
    Ok(format!(
        "status={} body={}",
        resp.status,
        String::from_utf8_lossy(&resp.body)
    ))
}

impl Guest for Plugin {
    fn render_block(_lang: String, _source: String, _theme: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }

    fn run_command(_id: String, _input: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }

    fn render_inline(_id: String, _matched: String) -> Result<String, String> {
        Err("unused".into())
    }

    /// document = URL to fetch; "twice:<url>" fetches it twice;
    /// "five:<url>" issues five fetches (limit probe).
    fn format_document(document: String) -> Result<String, String> {
        if let Some(url) = document.strip_prefix("five:") {
            for _ in 0..4 {
                fetch_one(url)?;
            }
            return fetch_one(url); // the fifth — host must reject
        }
        if let Some(url) = document.strip_prefix("twice:") {
            fetch_one(url)?;
            return fetch_one(url);
        }
        fetch_one(&document)
    }

    fn process_paste(_text: String) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// format "one" → single file; "many" → three files incl. a
    /// subdir; "evil" → a traversal path the host must reject.
    fn export_document(
        document: String,
        format: String,
        _theme: t::Theme,
    ) -> Result<Vec<ExportFile>, String> {
        let bytes = document.into_bytes();
        Ok(match format.as_str() {
            "one" => vec![ExportFile { path: "out.txt".into(), bytes }],
            "many" => vec![
                ExportFile { path: "index.html".into(), bytes: bytes.clone() },
                ExportFile { path: "assets/style.css".into(), bytes: bytes.clone() },
                ExportFile { path: "assets/app.js".into(), bytes },
            ],
            "evil" => vec![ExportFile { path: "../evil.txt".into(), bytes }],
            other => return Err(format!("unknown format {other}")),
        })
    }
}

export!(Plugin);
