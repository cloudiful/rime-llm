use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{fs, io::AsyncReadExt, io::AsyncWriteExt, time::timeout};
use tracing::info;

use crate::config::Settings;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("model download failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("model file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model download timed out")]
    Timeout,
    #[error("downloaded file is not a valid GGUF file")]
    InvalidGguf,
}

pub async fn ensure_model(settings: &Settings) -> Result<PathBuf, DownloadError> {
    let path = settings.model_path();
    if is_valid_gguf(&path).await? {
        info!(path = %path.display(), "using cached GGUF model");
        return Ok(path);
    }

    fs::create_dir_all(&settings.model_dir).await?;
    let temporary_path = temporary_path(&path);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()?;
    info!(url = %settings.model_url(), "downloading GGUF model");
    let download = async {
        let mut response = client.get(settings.model_url()).send().await?;
        response = response.error_for_status()?;
        let mut file = fs::File::create(&temporary_path).await?;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok::<(), DownloadError>(())
    };

    let download_result = timeout(
        Duration::from_secs(settings.download_timeout_secs),
        download,
    )
    .await;
    match download_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = fs::remove_file(&temporary_path).await;
            return Err(error);
        }
        Err(_) => {
            let _ = fs::remove_file(&temporary_path).await;
            return Err(DownloadError::Timeout);
        }
    }

    if !is_valid_gguf(&temporary_path).await? {
        let _ = fs::remove_file(&temporary_path).await;
        return Err(DownloadError::InvalidGguf);
    }
    fs::rename(&temporary_path, &path).await?;
    info!(path = %path.display(), "GGUF model download complete");
    Ok(path)
}

fn temporary_path(path: &Path) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_extension(format!("gguf.part.{}.{}", std::process::id(), suffix))
}

async fn is_valid_gguf(path: &Path) -> Result<bool, std::io::Error> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.len() < 1024 * 1024 {
        return Ok(false);
    }
    let mut file = fs::File::open(path).await?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).await?;
    Ok(&magic == b"GGUF")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_or_missing_files_are_rejected() {
        let path =
            std::env::temp_dir().join(format!("rime-llm-download-test-{}", std::process::id()));
        assert!(!is_valid_gguf(&path).await.unwrap());
    }
}
