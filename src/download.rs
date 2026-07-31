use std::path::{Path, PathBuf};

use hf_hub::api::tokio::{ApiBuilder, ApiError};
use tokio::{fs, io::AsyncReadExt, time::timeout};
use tracing::{info, warn};

use crate::config::Settings;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("model download failed: {0}")]
    HfHub(#[from] ApiError),
    #[error("model file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model download timed out")]
    Timeout,
    #[error("downloaded file is not a valid GGUF file")]
    InvalidGguf,
}

pub async fn ensure_model(settings: &Settings) -> Result<PathBuf, DownloadError> {
    let mut builder = ApiBuilder::from_env().with_progress(false);
    if let Some(cache_dir) = &settings.model_dir {
        builder = builder.with_cache_dir(cache_dir.clone());
    }
    let api = builder.build()?;

    info!(
        repo = %settings.model_repo,
        file = %settings.model_file,
        "resolving GGUF model via hf-hub"
    );
    let resolve = async {
        api.model(settings.model_repo.clone())
            .get(&settings.model_file)
            .await
    };

    let path = match timeout(
        std::time::Duration::from_secs(settings.download_timeout_secs),
        resolve,
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => return Err(DownloadError::Timeout),
    };

    if !is_valid_gguf(&path).await? {
        warn!(path = %path.display(), "cached GGUF is corrupt, removing and retrying");
        if let Ok(target) = fs::read_link(&path).await {
            let resolved = path.parent().map(|p| p.join(&target)).unwrap_or_else(|| target);
            let _ = fs::remove_file(resolved).await;
        }
        fs::remove_file(&path).await?;

        let retry = async {
            api.model(settings.model_repo.clone())
                .get(&settings.model_file)
                .await
        };
        let path = match timeout(
            std::time::Duration::from_secs(settings.download_timeout_secs),
            retry,
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => return Err(DownloadError::Timeout),
        };
        if !is_valid_gguf(&path).await? {
            return Err(DownloadError::InvalidGguf);
        }
        info!(path = %path.display(), "GGUF model ready after retry");
        return Ok(path);
    }
    info!(path = %path.display(), "GGUF model ready");
    Ok(path)
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
