use dioxus::prelude::*;
use std::fs::create_dir;

#[server]
async fn create_config_folder(path: String) -> Result<(), ServerFnError> {
    match create_dir(path) {
        Ok(()) => (),
        Err(e) => panic!("Error occured while trying to create folder .hestia"),
    }
    Ok(())
}
