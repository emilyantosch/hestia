use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WatchedFolderTree {
    name: String,
    path: String,
    children: Option<Vec<String>>,
    icon: Option<String>,
    color: Option<String>,
}

impl WatchedFolderTree {
    #[must_use]
    pub fn new(name: String, path: String) -> WatchedFolderTree {
        WatchedFolderTree {
            name,
            path,
            children: None,
            icon: None,
            color: None,
        }
    }

    #[must_use]
    pub fn with_icon_and_color(
        name: String,
        path: String,
        icon: Option<String>,
        color: Option<String>,
    ) -> WatchedFolderTree {
        WatchedFolderTree {
            name,
            path,
            children: None,
            icon,
            color,
        }
    }

    #[must_use]
    pub fn with(
        name: String,
        path: String,
        children: Option<Vec<String>>,
        icon: Option<String>,
        color: Option<String>,
    ) -> WatchedFolderTree {
        WatchedFolderTree {
            name,
            path,
            children,
            icon,
            color,
        }
    }

    #[must_use]
    pub fn children(mut self, children: Vec<String>) -> Self {
        self.children = Some(children);
        self
    }

    #[must_use]
    pub fn icon(mut self, icon: String) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn color(mut self, color: String) -> Self {
        self.color = Some(color);
        self
    }
}
