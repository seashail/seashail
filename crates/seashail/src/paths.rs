use base64::Engine as _;
use directories::ProjectDirs;
use eyre::{Context as _, ContextCompat as _};
use rand::Rng as _;
use std::path::PathBuf;
use std::{fs::OpenOptions, io::Write as _};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

#[derive(Debug, Clone)]
pub struct SeashailPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_file: PathBuf,
}

impl SeashailPaths {
    pub fn discover() -> eyre::Result<Self> {
        // Test/CI override knobs.
        if let (Ok(data_dir), Ok(config_dir)) = (
            std::env::var("SEASHAIL_DATA_DIR"),
            std::env::var("SEASHAIL_CONFIG_DIR"),
        ) {
            let data_dir = PathBuf::from(data_dir);
            let config_dir = PathBuf::from(config_dir);
            let log_file = data_dir.join("seashail.log.jsonl");
            return Ok(Self {
                config_dir,
                data_dir,
                log_file,
            });
        }

        // Default locations:
        // macOS: ~/Library/Application Support/seashail
        // Linux: ~/.config/seashail
        // Windows: %APPDATA%\\seashail
        let proj =
            ProjectDirs::from("", "", "seashail").context("failed to resolve project dirs")?;
        let config_dir = proj.config_dir().to_path_buf();
        let data_dir = proj.data_dir().to_path_buf();

        let log_file = data_dir.join("seashail.log.jsonl");

        Ok(Self {
            config_dir,
            data_dir,
            log_file,
        })
    }

    pub fn auth_token_path(&self) -> PathBuf {
        self.config_dir.join("daemon_auth_token.txt")
    }

    pub fn ensure_private_dirs(&self) -> eyre::Result<()> {
        crate::fsutil::ensure_private_dir(&self.config_dir)?;
        crate::fsutil::ensure_private_dir(&self.data_dir)?;
        Ok(())
    }

    pub fn ensure_auth_token(&self) -> eyre::Result<String> {
        self.ensure_private_dirs()?;
        let p = self.auth_token_path();
        let md_if_exists = || -> eyre::Result<Option<std::fs::Metadata>> {
            if !p.exists() {
                return Ok(None);
            }
            let md =
                std::fs::symlink_metadata(&p).with_context(|| format!("stat {}", p.display()))?;
            Ok(Some(md))
        };

        // Robust against crashes while creating the file:
        // Never create an empty visible final file. Instead, write a temp file and hard-link it
        // into place (fails if destination exists). If another process wins the race, read it.
        for _ in 0_usize..5_usize {
            if let Some(md) = md_if_exists()? {
                if md.file_type().is_symlink() {
                    eyre::bail!("refusing to read symlink: {}", p.display());
                }
                let s =
                    std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
                let tok = s.trim().to_owned();
                if !tok.is_empty() {
                    return Ok(tok);
                }
                // Empty token file is invalid (can happen if a process was killed mid-create).
                // Best-effort: remove and retry.
                drop(std::fs::remove_file(&p));
                continue;
            }

            let mut bytes = [0_u8; 32];
            rand::rng().fill_bytes(&mut bytes);
            let tok = base64::engine::general_purpose::STANDARD.encode(bytes);

            let parent = p
                .parent()
                .ok_or_else(|| eyre::eyre!("missing parent for {}", p.display()))?;
            let suffix = {
                let mut rand_bytes = [0_u8; 8];
                rand::rng().fill_bytes(&mut rand_bytes);
                hex::encode(rand_bytes)
            };
            let tmp = parent.join(format!(
                ".{}.tmp.{}",
                p.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("daemon_auth_token.txt"),
                suffix
            ));

            let mut oo = OpenOptions::new();
            oo.create_new(true).write(true).truncate(false);
            #[cfg(unix)]
            {
                oo.mode(crate::fsutil::MODE_FILE_PRIVATE);
            }
            let mut f = oo
                .open(&tmp)
                .with_context(|| format!("open temp {}", tmp.display()))?;
            f.write_all(format!("{tok}\n").as_bytes())
                .with_context(|| format!("write {}", tmp.display()))?;
            f.flush()
                .with_context(|| format!("flush {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("fsync {}", tmp.display()))?;
            drop(f);

            match std::fs::hard_link(&tmp, &p) {
                Ok(()) => {
                    drop(std::fs::remove_file(&tmp));
                    return Ok(tok);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Someone else created it; read and use theirs.
                    drop(std::fs::remove_file(&tmp));
                }
                Err(e) => {
                    drop(std::fs::remove_file(&tmp));
                    return Err(eyre::Report::new(e).wrap_err(format!(
                        "hard_link {} -> {}",
                        tmp.display(),
                        p.display()
                    )));
                }
            }
        }

        eyre::bail!("failed to create/read auth token file: {}", p.display())
    }
}
