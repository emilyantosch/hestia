use anyhow::{Context, Result};
use controllers::{AppController, FileInfo, TagInfo};
use tempfile::TempDir;

struct Sample {
    name: &'static str,
    content: &'static [u8],
    tags: &'static [&'static str],
    excerpt: &'static str,
}

const SAMPLES: &[Sample] = &[
    Sample {
        name: "Data encapsulation.png",
        content: include_bytes!("../../../test_vault/2024_10_03_data_capsulation_rev01.png"),
        tags: &["Learning", "Reference"],
        excerpt: "Keep the details inside.\nExpose only what matters.",
    },
    Sample {
        name: "Class composition.png",
        content: include_bytes!("../../../test_vault/2024_10_04_class_composition_rev01.png"),
        tags: &["Learning", "Reference"],
        excerpt: "Small pieces.\nBetter together.",
    },
    Sample {
        name: "Learning notes.md",
        content: b"# Learning notes\n\nMake the model simple.\nName things with care.\nBuild one idea at a time.\n",
        tags: &["Learning", "Notes"],
        excerpt: "Make the model simple.\nName things with care.\nBuild one idea at a time.",
    },
    Sample {
        name: "Project checklist.md",
        content: b"# Project checklist\n\n- Explore GPUI\n- Connect the library\n- Make space for discovery\n",
        tags: &["Notes", "To review"],
        excerpt: "01   Explore GPUI\n02   Connect the library\n03   Make space for discovery",
    },
    Sample {
        name: "Reading list.txt",
        content: b"A little inspiration\n\nThe Design of Everyday Things\nA Philosophy of Software Design\nThe Rust Programming Language\n",
        tags: &["Reference", "To review"],
        excerpt: "The Design of Everyday Things\nA Philosophy of Software Design\nThe Rust Programming Language",
    },
    Sample {
        name: "Ideas.md",
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
            _ => "Plain text",
        }
    }
}

pub(crate) struct DemoLibrary {
    pub files: Vec<DemoFile>,
    pub tags: Vec<TagInfo>,
    // Keep the backend and preview files alive; drop the controller before its directory.
    _controller: AppController,
    _directory: TempDir,
}

impl DemoLibrary {
    pub(crate) async fn load() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let content = directory.path().join("Learning library");
        std::fs::create_dir(&content)?;
        for sample in SAMPLES {
            std::fs::write(content.join(sample.name), sample.content)?;
        }
        let controller = AppController::new_in(directory.path().join("data"))?;
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
            tags,
            _controller: controller,
            _directory: directory,
        })
    }

    // ponytail: filter the six-file snapshot locally; use backend queries for real libraries.
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
    async fn demo_is_indexed_and_filters_by_name_and_tag() -> Result<()> {
        let library = DemoLibrary::load().await?;
        assert_eq!(library.files.len(), 6);
        assert_eq!(library.tags.len(), 4);
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
            .files
            .first()
            .context("Missing demo file")?
            .info
            .path()
            .parent()
            .context("Missing content folder")?
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
