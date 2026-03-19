mod common;

#[tokio::test(flavor = "multi_thread")]
async fn standalone_worker_creates_a_single_sqlite_store() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;
    let client = reqwest::Client::new();
    let info = client
        .get(&process.admin_url)
        .send()
        .await
        .expect("admin info");
    assert!(info.status().is_success());
    let info = info.text().await.expect("admin info body");
    let marker = "\"worker_sqlite_file\":\"";
    let start = info.find(marker).expect("worker sqlite file marker") + marker.len();
    let end = info[start..]
        .find('"')
        .map(|offset| start + offset)
        .expect("worker sqlite file terminator");
    let sqlite_file = &info[start..end];
    let sqlite_file = std::path::Path::new(sqlite_file);
    assert_eq!(
        sqlite_file.file_name().and_then(|name| name.to_str()),
        Some("standalone.sqlite3")
    );
    assert!(
        sqlite_file.is_file(),
        "sqlite file missing: {}",
        sqlite_file.display()
    );
    assert!(
        sqlite_file
            .parent()
            .expect("sqlite parent directory")
            .is_dir(),
        "sqlite dir missing: {}",
        sqlite_file
            .parent()
            .expect("sqlite parent directory")
            .display()
    );

    process.shutdown().await;
}
