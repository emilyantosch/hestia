use anyhow::{Context, Result, bail};
use library::library::{Library, LibraryConfig, LibraryPathConfig};
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn check_delete_library() -> Result<()> {
    use tempfile::TempDir;

    let path = dirs::data_dir().context("operating system did not provide a data directory")?;
    let test_path = TempDir::new_in(path)?;
    let lib = Library::new().switch_or_create_lib(test_path.path())?;
    tracing::info!("Found or created library to be deleted!");

    //Assert that path exists
    assert!(
        std::fs::exists(&test_path)?,
        "Library Path should exist before deletion"
    );

    //Delete Library
    if let Err(e) = lib.delete() {
        tracing::error!("The lib could not get deleted due to {e:#?}");
    }

    let still_exists = std::fs::exists(test_path)?;
    assert!(!still_exists);
    Ok(())
}

#[test]
fn check_library_creation_successful() -> Result<()> {
    let path = dirs::data_dir().context("operating system did not provide a data directory")?;
    let test_path = TempDir::new_in(path)?;
    let lib = Library::new().switch_or_create_lib(test_path.path())?;
    match lib.library_config.as_ref() {
        Some(conf) => {
            assert_eq!(conf.library_paths, vec![LibraryPathConfig::default()]);
        }
        None => {
            bail!("library configuration was not created");
        }
    }
    lib.delete()?;
    Ok(())
}

#[test]
fn check_library_default_values() -> Result<()> {
    let path = dirs::data_dir().context("operating system did not provide a data directory")?;
    let test_path = TempDir::new_in(path)?;
    let lib = Library::new().switch_or_create_lib(test_path.path())?;
    if let Some(lib_config) = lib.library_config.as_ref() {
        assert_eq!(lib_config, &LibraryConfig::default());
    }
    lib.delete()?;
    Ok(())
}

#[test]
fn check_library_write_save_and_retrieve() -> Result<()> {
    println!("Start of test");
    tracing::info!("Start of test");
    let path = dirs::data_dir().context("operating system did not provide a data directory")?;
    let test_path = TempDir::new_in(path)?;
    tracing::info!("Data home configured");
    let mut lib = Library::new().switch_or_create_lib(test_path.path())?;
    let extra_path = LibraryPathConfig {
        name: Some("Hello".to_string()),
        path: Some(PathBuf::new().join("home/emmi/Documents/")),
    };
    let lbc = vec![LibraryPathConfig::default(), extra_path.clone()];
    if let Some(lib_config) = lib.library_config.as_mut() {
        lib_config.library_paths.push(extra_path);
    }
    tracing::info!("Pushed lib paths, config is now: {lib:#?}");
    println!("Pushed lib paths, config is now: {lib:#?}");
    lib.save_config()?;
    let test_lib = Library::new().switch_or_create_lib(test_path.path())?;
    if let Some(lib_config) = test_lib.library_config.as_ref() {
        assert_eq!(lib_config.library_paths, lbc);
    }
    lib.delete()?;
    Ok(())
}
