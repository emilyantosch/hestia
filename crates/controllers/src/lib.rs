use anyhow::{Context, Result, anyhow, bail};
use entity::{file_has_tags, files, folders, tag_has_tags, tags};
use itertools::Itertools;
use library::library::{Library, LibraryConfig, LibraryPathConfig};
use migration::{Migrator, MigratorTrait};
use model::services::CanonPath;
use repositories::config::DatabaseSettings;
use repositories::fs::operations::FileRepository;
use repositories::manager::DatabaseManager;
use repositories::thumbnail::operations::ThumbnailOperations;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use services::fs::scanner::DirectoryScanner;
use services::fs::watcher::{FileWatcher, FileWatcherMessage, store_event};
use services::thumbnails::generator::ThumbnailGenerator;
use services::thumbnails::thumbnails::{ThumbnailProcessor, ThumbnailProcessorHandler};
use std::fmt::{self, Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryName(String);

impl LibraryName {
    fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let mut components = Path::new(value).components();
        if value.is_empty()
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            bail!("Library name must be a single folder name.");
        }
        Ok(Self(value.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for LibraryName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryInfo {
    name: LibraryName,
    path: PathBuf,
}

impl LibraryInfo {
    fn from_path(path: PathBuf) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("A library has an invalid folder name.")?;
        Ok(Self {
            name: LibraryName(name.to_string()),
            path,
        })
    }

    #[must_use]
    pub fn name(&self) -> &LibraryName {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderInfo {
    id: i32,
    name: String,
    path: PathBuf,
    parent_id: Option<i32>,
}

impl FolderInfo {
    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn parent_id(&self) -> Option<i32> {
        self.parent_id
    }
}

impl From<folders::Model> for FolderInfo {
    fn from(folder: folders::Model) -> Self {
        Self {
            id: folder.id,
            name: folder.name,
            path: folder.path.into(),
            parent_id: folder.parent_folder_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    id: i32,
    name: String,
    path: PathBuf,
    file_type_id: i32,
}

impl FileInfo {
    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn file_type_id(&self) -> i32 {
        self.file_type_id
    }

    #[must_use]
    pub fn thumbnail_path(&self) -> Option<&Path> {
        // ponytail: thumbnails are DB blobs; add a frontend image adapter before exposing paths.
        None
    }
}

impl From<files::Model> for FileInfo {
    fn from(file: files::Model) -> Self {
        Self {
            id: file.id,
            name: file.name,
            path: file.path.into(),
            file_type_id: file.file_type_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagInfo {
    id: i32,
    name: String,
    color: Option<u32>,
    sub_tag_ids: Vec<i32>,
}

impl TagInfo {
    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn color(&self) -> Option<u32> {
        self.color
    }

    #[must_use]
    pub fn sub_tag_ids(&self) -> &[i32] {
        &self.sub_tag_ids
    }
}

impl From<tags::Model> for TagInfo {
    fn from(tag: tags::Model) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            color: tag.color.map(|color| color as u32),
            sub_tag_ids: Vec::new(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ScanReport {
    files_scanned: usize,
    changed: usize,
}

impl ScanReport {
    #[must_use]
    pub fn files_scanned(self) -> usize {
        self.files_scanned
    }

    #[must_use]
    pub fn changed(self) -> usize {
        self.changed
    }
}

#[derive(Debug)]
struct Workspace {
    library_paths: Vec<CanonPath>,
    database_manager: Arc<DatabaseManager>,
    file_operations: Arc<FileRepository>,
    thumbnail_processor: ThumbnailProcessorHandler,
    // The watcher runs until this sender is dropped with the workspace.
    watcher: Option<mpsc::UnboundedSender<FileWatcherMessage>>,
}

#[derive(Debug)]
enum AppState {
    AwaitingLibrary,
    Ready {
        // Retain the library until deactivation: its Drop saves the last library.
        _library: Library,
        workspace: Workspace,
    },
}

#[derive(Debug)]
pub struct AppController {
    data_home: PathBuf,
    state: Mutex<AppState>,
}

impl AppController {
    pub fn new() -> Result<Self> {
        let data_home = library::io::create_or_validate_data_directory()
            .context("Could not open the application data directory")?;
        Self::new_in(data_home)
    }

    pub fn new_in(data_home: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(data_home.as_ref())
            .context("Could not open the application data directory")?;
        let data_home = data_home
            .as_ref()
            .canonicalize()
            .context("Could not open the application data directory")?;
        std::fs::create_dir_all(data_home.join("hestia"))
            .context("Could not open the application data directory")?;

        Ok(Self {
            data_home,
            state: Mutex::new(AppState::AwaitingLibrary),
        })
    }

    pub fn list_libraries(&self) -> Result<Vec<LibraryInfo>> {
        let mut libraries = Library::list_libraries_in(&self.data_home)
            .context("Could not list libraries")?
            .into_iter()
            .map(PathBuf::from)
            .map(LibraryInfo::from_path)
            .collect::<Result<Vec<_>>>()?;
        libraries.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        Ok(libraries)
    }

    pub async fn select_library(&self, path: impl AsRef<Path>) -> Result<LibraryInfo> {
        let path = path
            .as_ref()
            .canonicalize()
            .context("Could not open the library")?;
        let libraries_root = self.data_home.join("hestia");
        if !path.starts_with(&libraries_root)
            || !path.is_dir()
            || !path.join("config.toml").is_file()
        {
            bail!("The selected folder is not a Hestia library.");
        }

        let library = Library::new_in(&self.data_home)
            .switch_or_create_lib_in(&path, &self.data_home)
            .context("Could not open the library")?;
        self.activate(library).await
    }

    pub async fn create_library(
        &self,
        name: &str,
        content_path: impl AsRef<Path>,
    ) -> Result<LibraryInfo> {
        let name = LibraryName::parse(name)?;
        let content_path = content_path
            .as_ref()
            .canonicalize()
            .context("The library content path must be a folder.")?;
        if !content_path.is_dir() {
            bail!("The library content path must be a folder.");
        }

        let share_path = self.data_home.join("hestia").join(name.as_str());
        if share_path
            .try_exists()
            .context("Could not create the library")?
        {
            bail!("A library with that name already exists.");
        }

        let content_name = content_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Folder")
            .to_string();

        let config = LibraryConfig {
            name: name.to_string(),
            library_paths: vec![LibraryPathConfig {
                name: Some(content_name),
                path: Some(content_path),
            }],
            ..Default::default()
        };

        let mut library = Library::new_in(&self.data_home);
        library.share_path = Some(share_path);
        library.library_config = Some(config);
        library
            .save_config()
            .context("Could not create the library")?;

        self.activate(library).await
    }

    pub async fn initialize_workspace(&self) -> Result<()> {
        let (database_manager, file_operations, library_paths) = self
            .with_workspace(|workspace| {
                (
                    Arc::clone(&workspace.database_manager),
                    Arc::clone(&workspace.file_operations),
                    workspace.library_paths.clone(),
                )
            })
            .await?;

        Migrator::up(database_manager.get_connection().as_ref(), None)
            .await
            .context("database migration failed")
            .context("Could not initialize the library")?;
        if library_paths.is_empty() {
            bail!("The library has no content folders.");
        }
        file_operations
            .upsert_root_folders(library_paths)
            .await
            .context("Could not initialize the library")?;
        Ok(())
    }

    pub async fn scan(&self) -> Result<ScanReport> {
        let (file_operations, library_paths) = self
            .with_workspace(|workspace| {
                (
                    Arc::clone(&workspace.file_operations),
                    workspace.library_paths.clone(),
                )
            })
            .await?;

        let scanner = DirectoryScanner::new(file_operations);
        let mut result = ScanReport {
            files_scanned: 0,
            changed: 0,
        };
        for path in library_paths {
            let report = scanner
                .sync_directory(&path)
                .await
                .context("Could not scan the library")?;
            result.files_scanned += report.files_scanned;
            result.changed += report.changed();
        }
        Ok(result)
    }

    pub async fn generate_thumbnails(&self) -> Result<()> {
        self.with_workspace(|workspace| workspace.thumbnail_processor.queue_missing_files())
            .await?
            .context("Could not generate thumbnails")
    }

    pub async fn start_watching(&self) -> Result<mpsc::UnboundedReceiver<()>> {
        let (watcher_sender, watcher_receiver) = mpsc::unbounded_channel();
        let (database_manager, paths) = self
            .with_workspace(|workspace| {
                if workspace.watcher.is_some() {
                    return Err(anyhow::anyhow!("the library is already being watched"));
                }
                workspace.watcher = Some(watcher_sender.clone());
                Ok((
                    Arc::clone(&workspace.database_manager),
                    workspace.library_paths.clone(),
                ))
            })
            .await?
            .context("Could not watch the library folders")?;

        let (events_sender, mut events) = mpsc::channel(100);
        let (changes_sender, changes_receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if let Err(error) = FileWatcher::new(watcher_receiver).run(events_sender).await {
                tracing::error!(%error, "File watcher stopped");
            }
        });
        tokio::spawn(async move {
            let repository = FileRepository::new(database_manager);
            while let Some(event) = events.recv().await {
                match store_event(event, &repository).await {
                    Ok(()) => drop(changes_sender.send(())),
                    Err(error) => tracing::warn!("Failed to store event to database: {error:#}"),
                }
            }
        });
        for path in paths {
            watcher_sender
                .send(FileWatcherMessage::WatchPath(path))
                .context("Could not watch the library folders")?;
        }
        Ok(changes_receiver)
    }

    pub async fn list_folders(&self) -> Result<Vec<FolderInfo>> {
        let database_manager = self.database_manager().await?;
        folders::Entity::find()
            .order_by_asc(folders::Column::Name)
            .all(database_manager.get_connection().as_ref())
            .await
            .map(|items| items.into_iter().map_into().collect())
            .context("Could not query the library")
    }

    pub async fn list_files(&self, folder_id: Option<i32>, search: &str) -> Result<Vec<FileInfo>> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let mut condition = Condition::all();
        if let Some(folder_id) = folder_id {
            let folder = folders::Entity::find_by_id(folder_id)
                .one(connection.as_ref())
                .await
                .context("Could not query the library")?
                .context("The selected file no longer exists.")?;
            condition = condition.add(files::Column::Path.starts_with(folder.path));
        }
        let search = search.trim();
        if !search.is_empty() {
            condition = condition.add(
                Condition::any()
                    .add(files::Column::Name.contains(search))
                    .add(files::Column::Path.contains(search)),
            );
        }

        files::Entity::find()
            .filter(condition)
            .order_by_asc(files::Column::Name)
            .all(connection.as_ref())
            .await
            .map(|items| items.into_iter().map_into().collect())
            .context("Could not query the library")
    }

    pub async fn list_tags(&self) -> Result<Vec<TagInfo>> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let mut tags: Vec<TagInfo> = tags::Entity::find()
            .order_by_asc(tags::Column::Name)
            .all(connection.as_ref())
            .await
            .context("Could not query the library")?
            .into_iter()
            .map_into()
            .collect();
        let links = tag_has_tags::Entity::find()
            .all(connection.as_ref())
            .await
            .context("Could not query the library")?;
        for link in links {
            if let Some(tag) = tags.iter_mut().find(|tag| tag.id == link.super_tag_id) {
                tag.sub_tag_ids.push(link.sub_tag_id);
            }
        }
        Ok(tags)
    }

    pub async fn create_tag(&self, name: &str) -> Result<()> {
        let name = Self::tag_name(name)?;
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        if tags::Entity::find()
            .filter(tags::Column::Name.eq(name))
            .one(connection.as_ref())
            .await
            .context("Could not update tags")?
            .is_some()
        {
            return Ok(());
        }
        let now = chrono::Utc::now().naive_utc();
        tags::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            name: Set(name.to_string()),
            color: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(connection.as_ref())
        .await
        .map(|_| ())
        .context("Could not update tags")
    }

    pub async fn update_tag(
        &self,
        tag_id: i32,
        name: &str,
        color: Option<u32>,
        sub_tag_ids: &[i32],
    ) -> Result<()> {
        let name = Self::tag_name(name)?;
        if color.is_some_and(|color| color > 0x00ff_ffff) {
            bail!("Tag colors must be 24-bit RGB values.");
        }
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let transaction = connection.begin().await.context("Could not update tags")?;
        let tag = tags::Entity::find_by_id(tag_id)
            .one(&transaction)
            .await
            .context("Could not update tags")?
            .context("The selected tag no longer exists.")?;
        if tags::Entity::find()
            .filter(tags::Column::Name.eq(name))
            .filter(tags::Column::Id.ne(tag_id))
            .one(&transaction)
            .await
            .context("Could not update tags")?
            .is_some()
        {
            bail!("A tag with that name already exists.");
        }
        let links = tag_has_tags::Entity::find()
            .all(&transaction)
            .await
            .context("Could not update tags")?;
        let mut pending = sub_tag_ids.to_vec();
        let mut visited = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if id == tag_id {
                bail!("A tag cannot contain itself, directly or through other tags.");
            }
            if visited.insert(id) {
                pending.extend(
                    links
                        .iter()
                        .filter(|link| link.super_tag_id == id)
                        .map(|link| link.sub_tag_id),
                );
            }
        }
        let sub_tag_ids = sub_tag_ids.iter().copied().unique().collect_vec();
        let found_sub_tags = tags::Entity::find()
            .filter(tags::Column::Id.is_in(sub_tag_ids.clone()))
            .count(&transaction)
            .await
            .context("Could not update tags")?;
        if found_sub_tags != sub_tag_ids.len() as u64 {
            bail!("The selected tag no longer exists.");
        }
        let mut tag = tag.into_active_model();
        tag.name = Set(name.to_string());
        tag.color = Set(color.map(|color| color as i32));
        tag.updated_at = Set(chrono::Utc::now().naive_utc());
        tag.update(&transaction)
            .await
            .context("Could not update tags")?;
        tag_has_tags::Entity::delete_many()
            .filter(tag_has_tags::Column::SuperTagId.eq(tag_id))
            .exec(&transaction)
            .await
            .context("Could not update tags")?;
        for id in sub_tag_ids {
            tag_has_tags::ActiveModel {
                super_tag_id: Set(tag_id),
                sub_tag_id: Set(id),
                ..Default::default()
            }
            .insert(&transaction)
            .await
            .context("Could not update tags")?;
        }
        transaction.commit().await.context("Could not update tags")
    }

    pub async fn delete_tag(&self, tag_id: i32) -> Result<()> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let transaction = connection.begin().await.context("Could not update tags")?;
        file_has_tags::Entity::delete_many()
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .exec(&transaction)
            .await
            .context("Could not update tags")?;
        tag_has_tags::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(tag_has_tags::Column::SuperTagId.eq(tag_id))
                    .add(tag_has_tags::Column::SubTagId.eq(tag_id)),
            )
            .exec(&transaction)
            .await
            .context("Could not update tags")?;
        tags::Entity::delete_by_id(tag_id)
            .exec(&transaction)
            .await
            .context("Could not update tags")?;
        transaction.commit().await.context("Could not update tags")
    }

    pub async fn assign_tag(&self, file_id: i32, tag_id: i32) -> Result<()> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        if files::Entity::find_by_id(file_id)
            .one(connection.as_ref())
            .await
            .context("Could not update tags")?
            .is_none()
        {
            bail!("The selected file no longer exists.");
        }
        if tags::Entity::find_by_id(tag_id)
            .one(connection.as_ref())
            .await
            .context("Could not update tags")?
            .is_none()
        {
            bail!("The selected tag no longer exists.");
        }
        if file_has_tags::Entity::find()
            .filter(file_has_tags::Column::FileId.eq(file_id))
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .one(connection.as_ref())
            .await
            .context("Could not update tags")?
            .is_some()
        {
            return Ok(());
        }
        file_has_tags::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            file_id: Set(file_id),
            tag_id: Set(tag_id),
        }
        .insert(connection.as_ref())
        .await
        .map(|_| ())
        .context("Could not update tags")
    }

    pub async fn remove_tag(&self, file_id: i32, tag_id: i32) -> Result<()> {
        let database_manager = self.database_manager().await?;
        file_has_tags::Entity::delete_many()
            .filter(file_has_tags::Column::FileId.eq(file_id))
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .exec(database_manager.get_connection().as_ref())
            .await
            .map(|_| ())
            .context("Could not update tags")
    }

    /// Read from the open workspace without holding the state lock across awaits.
    async fn with_workspace<T>(&self, read: impl FnOnce(&mut Workspace) -> T) -> Result<T> {
        match &mut *self.state.lock().await {
            AppState::Ready { workspace, .. } => Ok(read(workspace)),
            AppState::AwaitingLibrary => Err(anyhow!("Select a library before initializing it.")),
        }
    }

    async fn database_manager(&self) -> Result<Arc<DatabaseManager>> {
        self.with_workspace(|workspace| Arc::clone(&workspace.database_manager))
            .await
    }

    fn tag_name(name: &str) -> Result<&str> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Tag names cannot be empty.");
        }
        Ok(name)
    }

    async fn activate(&self, library: Library) -> Result<LibraryInfo> {
        let info = LibraryInfo::from_path(
            library
                .share_path
                .clone()
                .context("The library has no storage folder.")?,
        )?;
        let workspace = Workspace::open(&library)
            .await
            .context("Could not open the library")?;
        *self.state.lock().await = AppState::Ready {
            _library: library,
            workspace,
        };
        Ok(info)
    }
}

impl Workspace {
    async fn open(library: &Library) -> Result<Self> {
        let library_paths = library
            .library_config
            .iter()
            .flat_map(|config| &config.library_paths)
            .filter_map(|root| root.path.clone())
            .map(CanonPath::try_from)
            .collect::<Result<Vec<_>>>()?;
        let database_path = library.get_canon_database_path()?;
        let connection_string = format!(
            "sqlite:///{}",
            database_path
                .as_str()
                .context("database path is not valid UTF-8")?
        );
        let settings = DatabaseSettings {
            con_string: connection_string,
            timeout: 30_000,
            ..DatabaseSettings::default()
        };
        let database_manager = Arc::new(DatabaseManager::new(settings).await?);
        database_manager.test_connection().await?;
        let file_operations = Arc::new(FileRepository::new(Arc::clone(&database_manager)));
        let (thumbnail_processor, receiver) = ThumbnailProcessorHandler::new();
        let processor = ThumbnailProcessor::new(
            receiver,
            Arc::new(ThumbnailOperations::new(Arc::clone(&database_manager))),
            Arc::new(ThumbnailGenerator::new()),
        );
        tokio::spawn(async move {
            if let Err(error) = processor.run().await {
                tracing::error!(%error, "Thumbnail processor stopped");
            }
        });
        Ok(Self {
            library_paths,
            database_manager,
            file_operations,
            thumbnail_processor,
            watcher: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AppController;
    use anyhow::{Context, Result, bail};
    use tempfile::TempDir;

    fn error_message<T>(result: Result<T>) -> Result<String> {
        match result {
            Ok(_) => bail!("expected an error"),
            Err(error) => Ok(error.to_string()),
        }
    }

    #[tokio::test]
    async fn create_library_returns_an_owned_summary() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let library = controller.create_library("Photos", content.path()).await?;

        assert_eq!(library.name().as_str(), "Photos");
        assert_eq!(
            library.path(),
            data_home.path().canonicalize()?.join("hestia/Photos")
        );
        assert!(library.path().join("config.toml").is_file());
        assert!(library.path().join("db.sqlite").is_file());
        Ok(())
    }

    #[tokio::test]
    async fn list_libraries_returns_only_library_directories() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Photos", content.path()).await?;
        std::fs::write(data_home.path().join("hestia/not-a-library"), "ignored")?;

        let libraries = controller.list_libraries()?;

        assert_eq!(libraries.len(), 1);
        assert_eq!(
            libraries
                .first()
                .context("the created library was not listed")?
                .name()
                .as_str(),
            "Photos"
        );
        Ok(())
    }

    #[tokio::test]
    async fn select_library_opens_an_existing_library() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let creator = AppController::new_in(data_home.path())?;
        let created = creator.create_library("Photos", content.path()).await?;
        let alias = data_home.path().join("content-alias");
        std::os::unix::fs::symlink(content.path(), &alias)?;
        std::fs::write(content.path().join("photo.txt"), "photo")?;
        {
            let mut state = creator.state.lock().await;
            let super::AppState::Ready {
                _library: library, ..
            } = &mut *state
            else {
                bail!("created library is not ready");
            };
            library
                .library_config
                .as_mut()
                .context("missing config")?
                .library_paths
                .first_mut()
                .context("missing root")?
                .path = Some(alias.clone());
            library.save_config()?;
        }
        let controller = AppController::new_in(data_home.path())?;
        let selected = controller.select_library(created.path()).await?;
        assert_eq!(selected, created);
        {
            let state = controller.state.lock().await;
            let super::AppState::Ready {
                _library: library,
                workspace,
            } = &*state
            else {
                bail!("selected library is not ready");
            };
            let configured_path = library
                .library_config
                .as_ref()
                .and_then(|config| config.library_paths.first())
                .and_then(|root| root.path.as_ref());
            assert_eq!(configured_path, Some(&alias));
            assert_eq!(
                workspace
                    .library_paths
                    .first()
                    .context("missing active root")?
                    .as_ref(),
                content.path().canonicalize()?
            );
        }
        controller.initialize_workspace().await?;
        controller.scan().await?;
        let files = controller.list_files(None, "photo").await?;
        assert_eq!(files.len(), 1);
        assert_eq!(
            files.first().context("missing scanned file")?.path(),
            content.path().canonicalize()?.join("photo.txt")
        );
        Ok(())
    }

    #[tokio::test]
    async fn initialize_workspace_migrates_the_selected_database() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;
        let library = controller.create_library("Photos", content.path()).await?;

        controller.initialize_workspace().await?;

        assert!(std::fs::metadata(library.path().join("db.sqlite"))?.len() > 0);
        Ok(())
    }

    #[tokio::test]
    async fn initialized_library_supports_the_frontend_controller_api() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        std::fs::write(content.path().join("notes.txt"), "hello")?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Notes", content.path()).await?;
        controller.initialize_workspace().await?;

        let report = controller.scan().await?;
        assert_eq!(report.files_scanned(), 1);
        let folders = controller.list_folders().await?;
        let folder_id = folders
            .first()
            .context("the scanned folder was not listed")?
            .id();
        let files = controller.list_files(Some(folder_id), "notes").await?;
        assert_eq!(files.len(), 1);
        let file_id = files
            .first()
            .context("the scanned file was not listed")?
            .id();

        controller.create_tag("Important").await?;
        let tags = controller.list_tags().await?;
        let tag = tags.first().context("the created tag was not listed")?;
        assert_eq!(tag.name(), "Important");
        let tag_id = tag.id();
        controller.assign_tag(file_id, tag_id).await?;
        controller.remove_tag(file_id, tag_id).await?;
        controller.delete_tag(tag_id).await?;
        assert!(controller.list_tags().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn renaming_preserves_links_and_removal_only_unlinks_one_file() -> Result<()> {
        use entity::file_has_tags;
        use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};

        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        std::fs::write(content.path().join("one.txt"), "one")?;
        std::fs::write(content.path().join("two.txt"), "two")?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Tags", content.path()).await?;
        controller.initialize_workspace().await?;
        controller.scan().await?;
        let files = controller.list_files(None, "").await?;
        let first = files.first().context("Missing first file")?.id();
        let second = files.get(1).context("Missing second file")?.id();
        controller.create_tag("Shared").await?;
        let id = controller
            .list_tags()
            .await?
            .first()
            .context("Missing tag")?
            .id();
        controller.assign_tag(first, id).await?;
        controller.assign_tag(second, id).await?;
        controller.create_tag("Taken").await?;
        assert_eq!(
            error_message(controller.update_tag(id, "  ", None, &[]).await)?,
            "Tag names cannot be empty."
        );
        assert_eq!(
            error_message(controller.update_tag(id, " Taken ", None, &[]).await)?,
            "A tag with that name already exists."
        );
        assert!(
            controller
                .list_tags()
                .await?
                .iter()
                .any(|tag| tag.id() == id && tag.name() == "Shared")
        );
        controller.update_tag(id, "  Renamed  ", None, &[]).await?;
        controller.update_tag(id, "Renamed", None, &[]).await?;
        assert!(
            controller
                .list_tags()
                .await?
                .iter()
                .any(|tag| tag.id() == id && tag.name() == "Renamed")
        );
        let connection = controller.database_manager().await?.get_connection();
        let links = file_has_tags::Entity::find().filter(file_has_tags::Column::TagId.eq(id));
        assert_eq!(links.clone().all(connection.as_ref()).await?.len(), 2);
        controller.remove_tag(first, id).await?;
        controller.remove_tag(first, id).await?;
        let remaining = links.clone().all(connection.as_ref()).await?;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining.first().context("Missing link")?.file_id, second);
        controller.remove_tag(second, id).await?;
        assert!(links.all(connection.as_ref()).await?.is_empty());
        assert_eq!(controller.list_tags().await?.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn tag_details_are_atomic_and_deletion_cleans_up_links() -> Result<()> {
        use entity::{file_has_tags, tag_has_tags};
        use sea_orm::EntityTrait as _;

        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        std::fs::write(content.path().join("one.txt"), "one")?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Tags", content.path()).await?;
        controller.initialize_workspace().await?;
        controller.scan().await?;
        for name in ["A", "B", "C"] {
            controller.create_tag(name).await?;
        }
        let tags = controller.list_tags().await?;
        let a = tags.first().context("A")?.id();
        let b = tags.get(1).context("B")?.id();
        let c = tags.get(2).context("C")?.id();
        let file_id = controller
            .list_files(None, "")
            .await?
            .first()
            .context("File")?
            .id();
        controller.assign_tag(file_id, b).await?;
        controller
            .update_tag(a, " A renamed ", Some(0x0070_baff), &[b, b])
            .await?;
        controller.update_tag(b, "B", None, &[c]).await?;
        let saved = controller.list_tags().await?;
        let tag = saved.iter().find(|tag| tag.id() == a).context("Saved A")?;
        assert_eq!(tag.name(), "A renamed");
        assert_eq!(tag.color(), Some(0x0070_baff));
        assert_eq!(tag.sub_tag_ids(), &[b]);
        assert_eq!(
            error_message(controller.update_tag(a, "Invalid", None, &[a]).await)?,
            "A tag cannot contain itself, directly or through other tags."
        );
        assert_eq!(
            error_message(controller.update_tag(c, "Invalid", None, &[a]).await)?,
            "A tag cannot contain itself, directly or through other tags."
        );
        assert_eq!(
            error_message(controller.update_tag(a, "Invalid", None, &[-1]).await)?,
            "The selected tag no longer exists."
        );
        assert_eq!(
            error_message(controller.update_tag(a, "B", None, &[]).await)?,
            "A tag with that name already exists."
        );
        assert_eq!(
            error_message(
                controller
                    .update_tag(a, "Invalid", Some(0x0100_0000), &[])
                    .await
            )?,
            "Tag colors must be 24-bit RGB values."
        );
        assert_eq!(controller.list_tags().await?, saved);
        controller.update_tag(a, "A renamed", None, &[]).await?;
        let cleared = controller.list_tags().await?;
        let tag = cleared
            .iter()
            .find(|tag| tag.id() == a)
            .context("Cleared A")?;
        assert_eq!(tag.color(), None);
        assert!(tag.sub_tag_ids().is_empty());
        controller
            .update_tag(a, "A renamed", Some(0x0070_baff), &[b])
            .await?;
        controller.delete_tag(b).await?;
        let remaining = controller.list_tags().await?;
        assert_eq!(remaining.len(), 2);
        assert!(
            remaining
                .iter()
                .all(|tag| tag.id() != b && tag.sub_tag_ids().is_empty())
        );
        assert_eq!(controller.list_files(None, "").await?.len(), 1);
        let connection = controller.database_manager().await?.get_connection();
        assert!(
            file_has_tags::Entity::find()
                .all(connection.as_ref())
                .await?
                .is_empty()
        );
        assert!(
            tag_has_tags::Entity::find()
                .all(connection.as_ref())
                .await?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn watcher_stores_created_files() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Watched", content.path()).await?;
        controller.initialize_workspace().await?;

        let mut changes = controller.start_watching().await?;
        assert!(controller.start_watching().await.is_err());
        // Let the watcher register the root before creating the file.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::fs::write(content.path().join("watched-entry.txt"), "new")?;
        tokio::time::timeout(std::time::Duration::from_secs(10), changes.recv())
            .await?
            .context("the watcher stopped")?;

        assert_eq!(controller.list_files(None, "watched-entry").await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn initialize_workspace_reports_when_no_library_is_selected() -> Result<()> {
        let data_home = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let Err(error) = controller.initialize_workspace().await else {
            bail!("initialization should require a selected library");
        };

        assert_eq!(
            error.to_string(),
            "Select a library before initializing it."
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_library_rejects_path_traversal_names() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let Err(error) = controller.create_library("../Photos", content.path()).await else {
            bail!("library names must not escape the library directory");
        };

        assert_eq!(
            error.to_string(),
            "Library name must be a single folder name."
        );
        Ok(())
    }
}
