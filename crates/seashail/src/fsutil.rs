use eyre::Context as _;
use rand::Rng as _;
use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

pub const MODE_DIR_PRIVATE: u32 = 0o700;
pub const MODE_FILE_PRIVATE: u32 = 0o600;

fn is_symlink(p: &Path) -> eyre::Result<bool> {
    let md = fs::symlink_metadata(p).with_context(|| format!("stat {}", p.display()))?;
    Ok(md.file_type().is_symlink())
}

pub fn ensure_private_dir(dir: &Path) -> eyre::Result<()> {
    if dir.exists() {
        if is_symlink(dir)? {
            eyre::bail!("refusing to use symlinked directory: {}", dir.display());
        }
        let md = fs::metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
        if !md.is_dir() {
            eyre::bail!("expected directory at {}", dir.display());
        }
    } else {
        fs::create_dir_all(dir).with_context(|| format!("create dir {}", dir.display()))?;
    }

    // Best-effort: enforce private perms on Unix.
    #[cfg(unix)]
    {
        let md = fs::metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
        let mut mode = md.permissions().mode();
        // If group/other have any bits set, clamp to 0700.
        if (mode & 0o077) != 0 {
            mode = MODE_DIR_PRIVATE;
            fs::set_permissions(dir, fs::Permissions::from_mode(mode))
                .with_context(|| format!("chmod {:o} {}", mode, dir.display()))?;
        }
    }

    Ok(())
}

fn tmp_path_for(parent: &Path, final_name: &Path) -> PathBuf {
    let base = final_name
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let mut rand_bytes = [0_u8; 8];
    rand::rng().fill_bytes(&mut rand_bytes);
    let suffix = hex::encode(rand_bytes);
    parent.join(format!(".{base}.tmp.{suffix}"))
}

pub fn write_atomic_restrictive(path: &Path, bytes: &[u8], mode: u32) -> eyre::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| eyre::eyre!("missing parent for {}", path.display()))?;
    ensure_private_dir(parent)?;

    if path.exists() && is_symlink(path)? {
        eyre::bail!("refusing to write to symlink: {}", path.display());
    }

    let tmp = tmp_path_for(parent, path);

    // Always create new temp files.
    let mut f = {
        #[cfg(unix)]
        {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(mode)
                .open(&tmp)
                .with_context(|| format!("open temp {}", tmp.display()))?
        }
        #[cfg(not(unix))]
        {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp)
                .with_context(|| format!("open temp {}", tmp.display()))?
        }
    };

    f.write_all(bytes)
        .with_context(|| format!("write {}", tmp.display()))?;
    f.flush()
        .with_context(|| format!("flush {}", tmp.display()))?;
    f.sync_all()
        .with_context(|| format!("fsync {}", tmp.display()))?;
    drop(f);

    // `rename` is atomic on Unix. On Windows, this can fail if the destination exists.
    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(path).with_context(|| format!("remove existing {}", path.display()))?;
        }
    }

    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;

    Ok(())
}

pub fn write_string_atomic_restrictive(path: &Path, s: &str, mode: u32) -> eyre::Result<()> {
    write_atomic_restrictive(path, s.as_bytes(), mode)
}
