use crate::components::{Alert, Echo, Hero};
use dioxus::prelude::*;
use dioxus_elements::div;
use std::fs::create_dir;

#[component]
pub fn Home() -> Element {
    let mut dir_created = use_signal(|| false);
    let mut file_names: Signal<Vec<String>> = use_signal(Vec::new);
    rsx! {
        button {
            class: "btn btn-primary",
            onclick: move |_| {
                // let _ = create_dir("~/projects/projects/hestia/.hestia");
                dir_created.set(true);
        },
            "Create .hestia folder"
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
