//! Times a full read of a generated vault: `cargo run --release -p tools
//! --example full_read [notes]` (default 5000).

use std::time::Instant;

use serde_json::json;
use tools::Tools;
use vault::Vault;

fn main() {
    let count: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5000);
    let dir = tempfile::tempdir().unwrap();
    for i in 0..count {
        let folder = dir.path().join(format!("Folder {:02}", i % 50));
        std::fs::create_dir_all(&folder).unwrap();
        let mut text = format!("---\naliases: [Alias {i}]\ntags: [gen/t{}, bulk]\nstatus: s{}\n---\n", i % 20, i % 7);
        for h in 0..5 {
            text.push_str(&format!("## Heading {h}\n"));
            text.push_str("Some prose about the topic with a #inline tag and words words words.\n");
            text.push_str(&format!("See [[Note {}]] and [[Note {}#Heading 1|that]] and ![[Note {}]].\n", (i * 7 + h) % count, (i + 13 * h) % count, (i + 1) % count));
            text.push_str("```\ncode [[not a link]]\n```\n");
            text.push_str(&"filler line of ordinary markdown text, a little longer.\n".repeat(5));
        }
        std::fs::write(folder.join(format!("Note {i}.md")), text).unwrap();
    }
    let vault = Vault::open(dir.path()).unwrap();
    let tools = Tools::new(vault.clone());
    let _ = vault.snapshot().unwrap(); // warm the page cache

    let time = |label: &str, f: &dyn Fn()| {
        let runs = 5;
        let mut best = f64::MAX;
        for _ in 0..runs {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{label:<40} best of {runs}: {best:7.1} ms");
    };
    time("snapshot (list + read + parse)", &|| {
        let _ = vault.snapshot().unwrap();
    });
    time("check_vault (snapshot + all links)", &|| {
        let _ = tools.call("check_vault", Some(json!({})));
    });
    time("get_links (snapshot + incoming)", &|| {
        let _ = tools.call("get_links", Some(json!({"note": "Note 42"})));
    });
    time("find_notes text", &|| {
        let _ = tools.call("find_notes", Some(json!({"text": "Heading 3"})));
    });
    println!("{count} notes");
}
