use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub size: u64,
    pub modified_unix_nanos: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    FreshFastPath,
    FreshAfterHash(SourceFingerprint),
    Stale(SourceFingerprint),
}

pub fn fingerprint(path: &Path) -> Result<SourceFingerprint> {
    let before = fs::metadata(path).map_err(|source| DotallError::io(path, source))?;
    let hash = hash_file(path)?;
    let after = fs::metadata(path).map_err(|source| DotallError::io(path, source))?;

    if metadata_tuple(&before)? != metadata_tuple(&after)? {
        return Err(DotallError::SourceChangedDuringRead(path.to_path_buf()));
    }

    Ok(SourceFingerprint {
        size: after.len(),
        modified_unix_nanos: modified_unix_nanos(&after)?,
        blake3: hash,
    })
}

pub fn check_freshness(path: &Path, expected: &SourceFingerprint) -> Result<Freshness> {
    let metadata = fs::metadata(path).map_err(|source| DotallError::io(path, source))?;
    if metadata.len() == expected.size
        && modified_unix_nanos(&metadata)? == expected.modified_unix_nanos
    {
        return Ok(Freshness::FreshFastPath);
    }

    let actual = fingerprint(path)?;
    if actual.blake3 == expected.blake3 {
        Ok(Freshness::FreshAfterHash(actual))
    } else {
        Ok(Freshness::Stale(actual))
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| DotallError::io(path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| DotallError::io(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn metadata_tuple(metadata: &Metadata) -> Result<(u64, u64)> {
    Ok((metadata.len(), modified_unix_nanos(metadata)?))
}

fn modified_unix_nanos(metadata: &Metadata) -> Result<u64> {
    let modified = metadata
        .modified()
        .map_err(|source| DotallError::io("<source metadata>", source))?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DotallError::InvalidWorkspacePath("<pre-epoch mtime>".into()))?;
    u64::try_from(duration.as_nanos())
        .map_err(|_| DotallError::InvalidWorkspacePath("<mtime overflow>".into()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use filetime::{FileTime, set_file_mtime};
    use tempfile::tempdir;

    use super::{Freshness, check_freshness, fingerprint};

    #[test]
    fn unchanged_metadata_uses_fast_path() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"same").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert_eq!(result, Freshness::FreshFastPath);
    }

    #[test]
    fn changed_content_is_stale() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"before").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");
        fs::write(&source, b"after and larger").expect("changed source");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert!(matches!(result, Freshness::Stale(_)));
    }

    #[test]
    fn metadata_only_change_rehashes_to_fresh() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"same").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");
        let changed_time = SystemTime::now() + Duration::from_secs(5);
        set_file_mtime(&source, FileTime::from_system_time(changed_time)).expect("set mtime");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert!(matches!(result, Freshness::FreshAfterHash(_)));
    }
}
