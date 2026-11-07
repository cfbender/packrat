//! Shared HTTP client helpers: a polite user agent and streamed downloads.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

pub const USER_AGENT: &str = concat!(
    "packrat/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/cfbender/packrat)"
);

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .expect("reqwest client")
}

/// Streams `url` into `path`, reporting progress as bytes arrive.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> anyhow::Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url}"))?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("create {}", path.display()))?;
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    let mut last_report = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("read {url}"))?;
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if received - last_report >= 4 * 1024 * 1024 {
            last_report = received;
            progress(received, total);
        }
    }
    file.flush().await?;
    progress(received, total);
    Ok(())
}

pub async fn get_bytes(client: &reqwest::Client, url: &str) -> anyhow::Result<bytes::Bytes> {
    Ok(client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url}"))?
        .bytes()
        .await?)
}
