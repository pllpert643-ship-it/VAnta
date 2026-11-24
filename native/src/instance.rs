//! Hold an OS file handle for the entire GUI lifetime; stale files never block restart.
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub struct InstanceGuard {
    _file: File,
}
impl InstanceGuard {
    pub fn acquire(directory: &Path) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|e| format!("Cannot create application data: {e}"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(directory.join("instance.lock"))
            .map_err(|e| format!("Cannot open application data exclusively. VAnta may already be running with this data directory. Close the other window and retry. ({e})"))?;
        #[cfg(not(windows))]
        file.try_lock()
            .map_err(|e| format!("VAnta is already using this data directory: {e}"))?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_instance_cannot_share_queue_but_restart_after_drop_succeeds() {
        let dir = std::env::temp_dir().join(format!("vanta-instance-{}", std::process::id()));
        let guard = InstanceGuard::acquire(&dir).unwrap();
        assert!(InstanceGuard::acquire(&dir).is_err());
        let other_dir = dir.join("isolated");
        let other = InstanceGuard::acquire(&other_dir).unwrap();
        drop(guard);
        let restarted = InstanceGuard::acquire(&dir).unwrap();
        drop(restarted);
        drop(other);
        fs::remove_file(other_dir.join("instance.lock")).unwrap();
        fs::remove_dir(other_dir).unwrap();
        fs::remove_file(dir.join("instance.lock")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
