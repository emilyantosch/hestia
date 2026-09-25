use anyhow::{Context, Result};
use controllers::{AppController, FileInfo, FolderInfo, TagInfo};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::{runtime::Handle, task::JoinHandle};

struct Sample {
    name: &'static str,
    folder: &'static str,
    content: &'static [u8],
    tags: &'static [&'static str],
    excerpt: &'static str,
}

const SAMPLES: &[Sample] = &[
    Sample {
        name: "Video sample.mp4",
        folder: "",
        content: include_bytes!("../../../test_vault/preview-demo.mp4"),
        tags: &[],
        excerpt: "Three-second test pattern with a quiet tone.\nPress Space to preview.",
    },
    Sample {
        name: "Data encapsulation.png",
        folder: "Images",
        content: include_bytes!("../../../test_vault/2024_10_03_data_capsulation_rev01.png"),
        tags: &["Learning", "Reference"],
        excerpt: "Keep the details inside.\nExpose only what matters.",
    },
    Sample {
        name: "Class composition.png",
        folder: "Images",
        content: include_bytes!("../../../test_vault/2024_10_04_class_composition_rev01.png"),
        tags: &["Learning", "Reference"],
        excerpt: "Small pieces.\nBetter together.",
    },
    Sample {
        name: "Learning notes.md",
        folder: "Documents/Notes",
        content: b"# Learning notes\n\nMake the model simple.\nName things with care.\nBuild one idea at a time.\n",
        tags: &["Learning", "Notes"],
        excerpt: "Make the model simple.\nName things with care.\nBuild one idea at a time.",
    },
    Sample {
        name: "Project checklist.md",
        folder: "Documents",
        content: b"# Project checklist\n\n- Explore GPUI\n- Connect the library\n- Make space for discovery\n",
        tags: &["Notes", "To review"],
        excerpt: "01   Explore GPUI\n02   Connect the library\n03   Make space for discovery",
    },
    Sample {
        name: "Reading list.txt",
        folder: "Documents",
        content: b"A little inspiration\n\nThe Design of Everyday Things\nA Philosophy of Software Design\nThe Rust Programming Language\n",
        tags: &["Reference", "To review"],
        excerpt: "The Design of Everyday Things\nA Philosophy of Software Design\nThe Rust Programming Language",
    },
    Sample {
        name: "Ideas.md",
        folder: "Documents/Notes",
        content: b"# A place for everything\n\nFind things by what they mean,\nnot just where they live.\n",
        tags: &["Notes"],
        excerpt: "Find things by what they mean,\nnot just where they live.",
    },
];

pub(crate) struct DemoFile {
    pub info: FileInfo,
    pub tags: Vec<i32>,
    pub excerpt: &'static str,
    pub bytes: usize,
}

impl DemoFile {
    pub(crate) fn kind(&self) -> &'static str {
        match self.info.path().extension().and_then(|ext| ext.to_str()) {
            Some("png") => "PNG image",
            Some("md") => "Markdown",
            Some("mp4") => "MP4 video",
            _ => "Plain text",
        }
    }
}

pub(crate) struct DemoLibrary {
    pub files: Vec<DemoFile>,
    pub folders: Vec<FolderInfo>,
    pub tags: Vec<TagInfo>,
    // Keep the backend and preview files alive; drop the controller before its directory.
    controller: Arc<AppController>,
    runtime: Handle,
    _directory: TempDir,
}

impl DemoLibrary {
    pub(crate) async fn load() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let content = directory.path().join("Learning library");
        std::fs::create_dir_all(content.join("Archive"))?;
        for sample in SAMPLES {
            let folder = content.join(sample.folder);
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join(sample.name), sample.content)?;
        }
        let controller = Arc::new(AppController::new_in(directory.path().join("data"))?);
        controller
            .create_library("Learning library", &content)
            .await?;
        controller.initialize_workspace().await?;
        controller.scan().await?;
        for name in ["Learning", "Notes", "Reference", "To review"] {
            controller.create_tag(name).await?;
        }
        let tags = controller.list_tags().await?;
        let mut files = Vec::new();
        for info in controller.list_files(None, "").await? {
            let sample = SAMPLES
                .iter()
                .find(|sample| sample.name == info.name())
                .context("Scanned file has no demo metadata")?;
            let mut assigned = Vec::new();
            for tag in &tags {
                if sample.tags.contains(&tag.name()) {
                    controller.assign_tag(info.id(), tag.id()).await?;
                    assigned.push(tag.id());
                }
            }
            files.push(DemoFile {
                info,
                tags: assigned,
                excerpt: sample.excerpt,
                bytes: sample.content.len(),
            });
        }
        Ok(Self {
            files,
            folders: controller.list_folders().await?,
            tags,
            controller,
            runtime: Handle::current(),
            _directory: directory,
        })
    }

    pub(crate) fn file_location(&self, file: &DemoFile) -> String {
        self.folders
            .iter()
            .find(|folder| {
                folder.parent_id().is_none() && file.info.path().starts_with(folder.path())
            })
            .and_then(|folder| folder.path().parent())
            .and_then(|root| file.info.path().strip_prefix(root).ok())
            .unwrap_or(file.info.path())
            .display()
            .to_string()
    }

    pub(crate) fn add_tag(
        &self,
        file_id: i32,
        name: String,
    ) -> JoinHandle<Result<(Vec<TagInfo>, i32)>> {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            controller.create_tag(&name).await?;
            let tags = controller.list_tags().await?;
            let tag_id = tags
                .iter()
                .find(|tag| tag.name() == name.trim())
                .context("Created tag was not found")?
                .id();
            controller.assign_tag(file_id, tag_id).await?;
            Ok((tags, tag_id))
        })
    }

    pub(crate) fn create_tag(&self, name: String) -> JoinHandle<Result<Vec<TagInfo>>> {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            controller.create_tag(&name).await?;
            Ok(controller.list_tags().await?)
        })
    }

    pub(crate) fn remove_tag(&self, file_id: i32, tag_id: i32) -> JoinHandle<Result<()>> {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            controller.remove_tag(file_id, tag_id).await?;
            Ok(())
        })
    }

    pub(crate) fn update_tag(
        &self,
        tag_id: i32,
        name: String,
        color: Option<u32>,
        sub_tag_ids: Vec<i32>,
    ) -> JoinHandle<Result<Vec<TagInfo>>> {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            controller
                .update_tag(tag_id, &name, color, &sub_tag_ids)
                .await?;
            Ok(controller.list_tags().await?)
        })
    }

    pub(crate) fn delete_tag(&self, tag_id: i32) -> JoinHandle<Result<Vec<TagInfo>>> {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            controller.delete_tag(tag_id).await?;
            Ok(controller.list_tags().await?)
        })
    }

    // ponytail: filter the demo snapshot locally; use backend queries for real libraries.
    pub(crate) fn visible_files(&self, tag: Option<i32>, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                let matches_tag = tag.is_none_or(|id| file.tags.contains(&id));
                let matches_query = file.info.name().to_lowercase().contains(&query)
                    || self.tags.iter().any(|tag| {
                        file.tags.contains(&tag.id()) && tag.name().to_lowercase().contains(&query)
                    });
                (matches_tag && matches_query).then_some(index)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tags_can_be_created_and_reused_with_validation() -> Result<()> {
        let library = DemoLibrary::load().await?;
        let file_id = library.files.first().context("Missing file")?.info.id();
        assert!(library.add_tag(file_id, "  ".into()).await?.is_err());
        let (tags, notes_id) = library.add_tag(file_id, "Notes".into()).await??;
        assert_eq!(tags.len(), 4);
        assert!(
            tags.iter()
                .any(|tag| tag.id() == notes_id && tag.name() == "Notes")
        );
        let (tags, tag_id) = library.add_tag(file_id, "  Personal  ".into()).await??;
        assert_eq!(tags.len(), 5);
        assert!(
            tags.iter()
                .any(|tag| tag.id() == tag_id && tag.name() == "Personal")
        );
        let (tags, duplicate_id) = library.add_tag(file_id, "Personal".into()).await??;
        assert_eq!(duplicate_id, tag_id);
        assert_eq!(tags.len(), 5);
        assert_eq!(library.controller.list_tags().await?.len(), 5);
        assert!(library.add_tag(-1, "Notes".into()).await?.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn demo_is_indexed_and_filters_by_name_and_tag() -> Result<()> {
        let library = DemoLibrary::load().await?;
        assert_eq!(library.files.len(), SAMPLES.len());
        assert_eq!(library.tags.len(), 4);
        assert_eq!(library.folders.len(), 5, "Includes the root and empty Archive");
        for sample in SAMPLES {
            let file = library
                .files
                .iter()
                .find(|file| file.info.name() == sample.name)
                .context("Missing sample file")?;
            assert_eq!(
                library.file_location(file),
                std::path::Path::new("Learning library")
                    .join(sample.folder)
                    .join(sample.name)
                    .display()
                    .to_string()
            );
        }
        let notes = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Notes")
            .context("Missing Notes tag")?;
        assert_eq!(library.visible_files(Some(notes.id()), "").len(), 3);
        assert_eq!(library.visible_files(None, "  LEARNING  ").len(), 3);
        assert_eq!(library.visible_files(Some(notes.id()), "PROJECT").len(), 1);
        assert!(library.visible_files(Some(notes.id()), "png").is_empty());
        assert!(library.visible_files(None, "no such file").is_empty());
        let root = library
            .folders
            .iter()
            .find(|folder| folder.parent_id().is_none())
            .context("Missing library root")?
            .path()
            .parent()
            .context("Missing temporary directory")?
            .to_path_buf();
        assert!(
            library
                .files
                .iter()
                .all(|file| file.info.path().starts_with(&root))
        );
        drop(library);
        assert!(!root.exists());
        Ok(())
    }
}
