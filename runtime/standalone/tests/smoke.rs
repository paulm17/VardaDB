mod common;

#[tokio::test(flavor = "multi_thread")]
async fn standalone_smoke_starts_and_shuts_down_cleanly() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;
    process.shutdown().await;
}
