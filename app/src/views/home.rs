use crate::components::{Echo, Hero};
use crate::file_system::watcher::FileWatcher;
use dioxus::prelude::*;
use std::fs::{create_dir, File};
use std::path::PathBuf;

#[component]
pub fn Home() -> Element {
    let mut dir_created = use_signal(|| false);
    let mut file_names: Signal<Vec<String>> = use_signal(Vec::new);
    rsx! {
        button {
            class: "btn btn-primary",
            onclick: async move |_| {
                watch_folder().await;
        },
            "Create .hestia folder"
        }
        button {
            class: "btn btn-primary",
            onclick: move |_| async move {
                let new_vault_path = rfd::FileDialog::new().set_directory("~/").pick_folder();

                let result_path = create_config_folder(new_vault_path
                                                        .unwrap()
                                                        .to_str()
                                                        .unwrap()
                                                        .to_string() + "/.hestia/")
                    .expect("Could not create vault folder");

                create_database_file(result_path + "/db.sqlite")
                    .expect("Could not create database file");

        },
            "New Vault"
        }
    if dir_created() {
        input {
            r#type: "file",
            accept: ".txt,.rs",
            multiple: true}
    }
        Hero {}
        Echo {}
    }
}

async fn watch_folder() {
    let mut watcher = FileWatcher::new().await.unwrap();
    watcher.init_watcher().await;
    watcher
        .watch(&PathBuf::from(
            r"/home/emmi/projects/projects/hestia/test_vault/",
        ))
        .await
        .unwrap();
}

fn create_config_folder(path: String) -> Result<String, ServerFnError> {
    match create_dir(path.clone()) {
        Ok(()) => (),
        Err(e) => println!("{:?}", e),
    };
    Ok(path)
}

fn create_database_file(path: String) -> Result<(), ServerFnError> {
    File::create(path)?;
    Ok(())
}
