use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone)]
pub struct Krate {
    name: String,
    version: String,
    dependents: Vec<Krate>,
}

impl Krate {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            dependents: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn dependents(&self) -> &Vec<Krate> {
        &self.dependents
    }

    pub fn dependents_mut(&mut self) -> &mut Vec<Krate> {
        &mut self.dependents
    }

    /// obtain the download directory
    /// $DOWNLOAD_DIR/crate_name/ ,such as /home/rust/xinshi/download/crossbeam-channel/
    fn get_download_dir(&self) -> PathBuf {
        let base_dir = std::env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "./downloads".to_string());
        Path::new(&base_dir).join(&self.name)
    }

    /// obtain the crate file path
    /// $DOWNLOAD_DIR/crate_name/crate_name-crate_version.crate
    fn get_crate_file_path(&self) -> PathBuf {
        let crate_file = format!("{}-{}.crate", self.name, self.version);
        self.get_download_dir().join(crate_file)
    }

    /// obtain the extract directory path
    /// $DOWNLOAD_DIR/crate_name/crate_name-crate_version/
    fn get_extract_dir_path(&self) -> PathBuf {
        let extract_dir = format!("{}-{}", self.name, self.version);
        self.get_download_dir().join(extract_dir)
    }

    /// download the crate file
    async fn download(&self) -> Result<()> {
        info!("download crate: {} {}", self.name, self.version);

        let download_dir = self.get_download_dir();
        let crate_file_path = self.get_crate_file_path();
        let extract_dir_path = self.get_extract_dir_path();

        // check if the crate-version.crate file already exists
        // we don't need to download the crate file again
        if crate_file_path.exists() {
            info!(
                "directory {} already exists, skip the download",
                extract_dir_path.display()
            );
            return Ok(());
        }

        tokio_fs::create_dir_all(&download_dir)
            .await
            .context(format!(
                "Failed to create the download directory: {}",
                download_dir.display()
            ))?;

        // download the crate file
        info!("downloading the crate file: {}", crate_file_path.display());
        let download_url = format!(
            "https://crates.io/api/v1/crates/{}/{}/download",
            self.name, self.version
        );

        let download_result = Command::new("curl")
            .args(&[
                "-L",
                &download_url,
                "-o",
                &crate_file_path.to_string_lossy(),
            ])
            .output()
            .await;

        if let Err(e) = download_result {
            return Err(anyhow::anyhow!("Failed to download the crate: {}", e));
        }

        // check the file size
        let metadata = tokio_fs::metadata(&crate_file_path).await.context(format!(
            "Failed to get the file metadata: {}",
            crate_file_path.display()
        ))?;

        let size = metadata.len();
        info!("the size of the downloaded file is {} bytes", size);

        if size == 0 {
            return Err(anyhow::anyhow!(
                "the size of the downloaded file is 0, maybe the download failed"
            ));
        }

        Ok(())
    }

    /// unzip the crate file
    async fn unzip(&self) -> Result<PathBuf> {
        let crate_file_path = self.get_crate_file_path();
        let extract_dir_path = self.get_extract_dir_path();
        let download_dir = self.get_download_dir();

        // if the target directory already exists, return directly
        if extract_dir_path.exists() {
            info!(
                "directory {} already exists, no need to extract",
                extract_dir_path.display()
            );
            return Ok(extract_dir_path);
        }

        // ensure the crate file exists
        if !crate_file_path.exists() {
            return Err(anyhow::anyhow!(
                "Cannot extract, crate file does not exist: {}",
                crate_file_path.display()
            ));
        }

        // extract the crate
        info!(
            "extracting crate: {} to {}",
            crate_file_path.display(),
            download_dir.display()
        );

        let unzip_result = Command::new("tar")
            .args(&["-xf", &crate_file_path.to_string_lossy()])
            .current_dir(&download_dir)
            .output()
            .await
            .context("Failed to execute tar command")?;

        if !unzip_result.status.success() {
            let stderr = String::from_utf8_lossy(&unzip_result.stderr);
            return Err(anyhow::anyhow!("Extract command failed: {}", stderr));
        }

        // check if the directory exists
        if !extract_dir_path.exists() {
            // try to list the current directory contents
            let entries = tokio_fs::read_dir(&download_dir)
                .await
                .context("Failed to read directory")?;

            let mut files = String::new();
            let mut entry_count = 0;

            tokio::pin!(entries);
            while let Some(entry) = entries
                .next_entry()
                .await
                .context("Failed to read directory entry")?
            {
                files.push_str(&format!("\n  - {}", entry.path().display()));
                entry_count += 1;

                if entry_count > 10 {
                    files.push_str("\n  ... (more files)");
                    break;
                }
            }

            return Err(anyhow::anyhow!(
                "Extracted directory does not exist: {}. Directory contents: {}",
                extract_dir_path.display(),
                files
            ));
        }

        info!(
            "Successfully extracted crate to: {}",
            extract_dir_path.display()
        );
        Ok(extract_dir_path)
    }

    /// download and unzip the crate, return the path to the extracted directory
    pub async fn get_crate_dir_path(&self) -> Result<PathBuf> {
        let extract_dir_path = self.get_extract_dir_path();

        // 优先判断解压目录是否已存在
        if extract_dir_path.exists() && extract_dir_path.is_dir() {
            return Ok(extract_dir_path);
        }

        // 如果没有解压目录，才需要下载和解压
        self.download().await?;
        let unzip_path = self.unzip().await?;

        // 检查解压目录
        if !unzip_path.is_dir() || unzip_path.read_dir().is_err() {
            return Err(anyhow::anyhow!(
                "the unzip path is not a directory: {}",
                unzip_path.display()
            ));
        }

        Ok(unzip_path)
    }

    /// cleanup the downloaded crate file, keep the extracted directory
    pub async fn cleanup_crate_file(&self) -> Result<()> {
        let crate_file_path = self.get_crate_file_path();

        if crate_file_path.exists() {
            tokio_fs::remove_file(&crate_file_path)
                .await
                .context(format!(
                    "Failed to delete file: {}",
                    crate_file_path.display()
                ))?;
            info!("Deleted crate file: {}", crate_file_path.display());
        }

        Ok(())
    }
}
