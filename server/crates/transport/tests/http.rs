//! The binary's HTTP face: stateless POSTs to /mcp, no initialize first.

use serde_json::{Value, json};
use tools::Tools;
use vault::Vault;

async fn serve(dir: &std::path::Path) -> String {
    let app = transport::router(Tools::new(Vault::open(dir).unwrap()), &[]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/mcp")
}

async fn post(url: &str, body: Value) -> (reqwest::StatusCode, String, Value) {
    let resp = reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let ct = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    (status, ct, resp.json().await.unwrap())
}

#[tokio::test]
async fn list_and_call_without_initialize() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Alpha.md"), "# Alpha\n[[Beta]]\n").unwrap();
    std::fs::write(dir.path().join("Beta.md"), "back to [[Alpha]]\n").unwrap();
    let url = serve(dir.path()).await;

    let (status, ct, v) = post(&url, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("application/json"), "{ct}");
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec!["find_notes", "read_note", "get_links", "check_vault", "create_note", "edit_section", "set_property", "move_note", "delete_note"]
    );

    let (status, _, v) = post(
        &url,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "read_note", "arguments": {"note": "alpha"}}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(v["result"]["isError"], false);
    let inner: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(inner["path"], "Alpha.md");
    assert_eq!(inner["content"], "# Alpha\n[[Beta]]\n");

    let (_, _, v) = post(
        &url,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "read_note", "arguments": {"note": ".git/config"}}}),
    )
    .await;
    assert_eq!(v["result"]["isError"], true);
    let inner: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(inner["error"], "outside_vault");
}
