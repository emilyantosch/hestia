use anyhow::Result;
use hash::{ContentDigest, FilesystemObjectId};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;

#[tokio::test]
async fn content_digest_is_blake3_of_file_bytes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    let changed = directory.path().join("changed");
    tokio::fs::write(&first, b"same bytes").await?;
    tokio::fs::write(&second, b"same bytes").await?;
    tokio::fs::write(&changed, b"different bytes").await?;

    let first = ContentDigest::observe(&first).await?;
    let second = ContentDigest::observe(&second).await?;
    let changed = ContentDigest::observe(&changed).await?;

    assert_eq!(first, second);
    assert_ne!(first, changed);
    assert_eq!(first.as_bytes(), blake3::hash(b"same bytes").as_bytes());
    Ok(())
}

#[tokio::test]
async fn content_digest_rejects_unsupported_entries() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("file");
    let symlink = directory.path().join("symlink");
    let socket = directory.path().join("socket");
    tokio::fs::write(&file, b"bytes").await?;
    std::os::unix::fs::symlink(&file, &symlink)?;
    let _listener = UnixListener::bind(&socket)?;

    for unsupported in [directory.path(), symlink.as_path(), socket.as_path()] {
        assert!(ContentDigest::observe(unsupported).await.is_err());
    }
    Ok(())
}

#[tokio::test]
async fn filesystem_object_id_survives_rename_and_is_shared_by_hard_links() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let original = directory.path().join("original");
    let renamed = directory.path().join("renamed");
    let hard_link = directory.path().join("hard-link");
    tokio::fs::write(&original, b"bytes").await?;

    let before = FilesystemObjectId::observe(&original).await?;
    tokio::fs::rename(&original, &renamed).await?;
    let after = FilesystemObjectId::observe(&renamed).await?;
    tokio::fs::hard_link(&renamed, &hard_link).await?;
    let linked = FilesystemObjectId::observe(&hard_link).await?;

    let metadata = std::fs::symlink_metadata(&renamed)?;
    assert_eq!(before, after);
    assert_eq!(before, linked);
    assert_eq!(before.device, metadata.dev());
    assert_eq!(before.inode, metadata.ino());
    Ok(())
}
