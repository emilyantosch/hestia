use anyhow::{Context, Result};
use entity::{file_has_tags, files, folders, tags};
use library::library::{Library, LibraryConfig, LibraryPathConfig};
use migration::{Migrator, MigratorTrait};
use model::services::CanonPath;
use repositories::config::DatabaseSettings;
use repositories::fs::operations::FileRepository;
use repositories::manager::DatabaseManager;
use repositories::thumbnail::operations::ThumbnailOperations;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use services::fs::scanner::DirectoryScanner;
use services::fs::watcher::{DatabaseFileWatcherEventHandler, FileWatcher, FileWatcherMessage};
use services::thumbnails::generator::ThumbnailGenerator;
use services::thumbnails::thumbnails::{ThumbnailProcessor, ThumbnailProcessorHandler};
use std::fmt::{self, Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryName(String);

impl LibraryName {
    fn parse(value: &str) -> ControllerResult<Self> {
        let value = value.trim();
        let mut components = Path::new(value).components();
        if value.is_empty()
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(ControllerError::InvalidLibraryName);
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
    fn from_path(path: PathBuf) -> ControllerResult<Self> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(ControllerError::InvalidLibraryFolderName)?;
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
        // ponytail: thumbnails are DB blobs; add a QML image provider before exposing URLs.
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
}

impl From<tags::Model> for TagInfo {
    fn from(tag: tags::Model) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControllerOperation {
    OpenDataDirectory,
    ListLibraries,
    OpenLibrary,
    CreateLibrary,
    InitializeLibrary,
    QueryLibrary,
    ScanLibrary,
    StartWatcher,
    GenerateThumbnails,
    ManageTags,
}

impl ControllerOperation {
    fn action(self) -> &'static str {
        match self {
            Self::OpenDataDirectory => "Could not open the application data directory",
            Self::ListLibraries => "Could not list libraries",
            Self::OpenLibrary => "Could not open the library",
            Self::CreateLibrary => "Could not create the library",
            Self::InitializeLibrary => "Could not initialize the library",
            Self::QueryLibrary => "Could not query the library",
            Self::ScanLibrary => "Could not scan the library",
            Self::StartWatcher => "Could not watch the library folders",
            Self::GenerateThumbnails => "Could not generate thumbnails",
            Self::ManageTags => "Could not update tags",
        }
    }
}

#[derive(Debug)]
pub enum ControllerError {
    InvalidLibraryName,
    InvalidLibraryFolderName,
    NotHestiaLibrary,
    InvalidContentFolder,
    LibraryAlreadyExists,
    NoLibrarySelected,
    MissingStorageFolder,
    NoContentFolders,
    InvalidTagName,
    FileNotFound,
    TagNotFound,
    OperationFailed {
        operation: ControllerOperation,
        source: anyhow::Error,
    },
}

impl ControllerError {
    fn operation(operation: ControllerOperation, source: impl Into<anyhow::Error>) -> Self {
        Self::OperationFailed {
            operation,
            source: source.into(),
        }
    }
}

impl Display for ControllerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLibraryName => {
                formatter.write_str("Library name must be a single folder name.")
            }
            Self::InvalidLibraryFolderName => {
                formatter.write_str("A library has an invalid folder name.")
            }
            Self::NotHestiaLibrary => {
                formatter.write_str("The selected folder is not a Hestia library.")
            }
            Self::InvalidContentFolder => {
                formatter.write_str("The library content path must be a folder.")
            }
            Self::LibraryAlreadyExists => {
                formatter.write_str("A library with that name already exists.")
            }
            Self::NoLibrarySelected => {
                formatter.write_str("Select a library before initializing it.")
            }
            Self::MissingStorageFolder => formatter.write_str("The library has no storage folder."),
            Self::NoContentFolders => formatter.write_str("The library has no content folders."),
            Self::InvalidTagName => formatter.write_str("Tag names cannot be empty."),
            Self::FileNotFound => formatter.write_str("The selected file no longer exists."),
            Self::TagNotFound => formatter.write_str("The selected tag no longer exists."),
            Self::OperationFailed { operation, source } => {
                write!(formatter, "{}: {source:#}", operation.action())
            }
        }
    }
}

impl std::error::Error for ControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OperationFailed { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

type ControllerResult<T> = std::result::Result<T, ControllerError>;

#[derive(Debug)]
struct Workspace {
    database_manager: Arc<DatabaseManager>,
    file_operations: Arc<FileRepository>,
    thumbnail_processor: ThumbnailProcessorHandler,
    watcher_started: bool,
}

#[derive(Debug)]
enum AppState {
    AwaitingLibrary,
    Ready {
        library: Library,
        workspace: Workspace,
    },
}

#[derive(Debug)]
pub struct AppController {
    data_home: PathBuf,
    state: Mutex<AppState>,
}

impl AppController {
    pub fn new() -> ControllerResult<Self> {
        let data_home = library::io::create_or_validate_data_directory().map_err(|error| {
            ControllerError::operation(ControllerOperation::OpenDataDirectory, error)
        })?;
        Self::new_in(data_home)
    }

    pub fn new_in(data_home: impl AsRef<Path>) -> ControllerResult<Self> {
        std::fs::create_dir_all(data_home.as_ref()).map_err(|error| {
            ControllerError::operation(ControllerOperation::OpenDataDirectory, error)
        })?;
        let data_home = data_home.as_ref().canonicalize().map_err(|error| {
            ControllerError::operation(ControllerOperation::OpenDataDirectory, error)
        })?;
        std::fs::create_dir_all(data_home.join("hestia")).map_err(|error| {
            ControllerError::operation(ControllerOperation::OpenDataDirectory, error)
        })?;

        Ok(Self {
            data_home,
            state: Mutex::new(AppState::AwaitingLibrary),
        })
    }

    pub fn list_libraries(&self) -> ControllerResult<Vec<LibraryInfo>> {
        let mut libraries = Library::list_libraries_in(&self.data_home)
            .map_err(|error| ControllerError::operation(ControllerOperation::ListLibraries, error))?
            .into_iter()
            .map(PathBuf::from)
            .map(LibraryInfo::from_path)
            .collect::<ControllerResult<Vec<_>>>()?;
        libraries.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        Ok(libraries)
    }

    pub async fn select_library(&self, path: impl AsRef<Path>) -> ControllerResult<LibraryInfo> {
        let path = path
            .as_ref()
            .canonicalize()
            .map_err(|error| ControllerError::operation(ControllerOperation::OpenLibrary, error))?;
        let libraries_root = self.data_home.join("hestia");
        if !path.starts_with(&libraries_root)
            || !path.is_dir()
            || !path.join("config.toml").is_file()
        {
            return Err(ControllerError::NotHestiaLibrary);
        }

        let library = Library::new_in(&self.data_home)
            .switch_or_create_lib_in(&path, &self.data_home)
            .map_err(|error| ControllerError::operation(ControllerOperation::OpenLibrary, error))?;
        self.activate(library).await
    }

    pub async fn create_library(
        &self,
        name: &str,
        content_path: impl AsRef<Path>,
    ) -> ControllerResult<LibraryInfo> {
        let name = LibraryName::parse(name)?;
        let content_path = content_path
            .as_ref()
            .canonicalize()
            .map_err(|_| ControllerError::InvalidContentFolder)?;
        if !content_path.is_dir() {
            return Err(ControllerError::InvalidContentFolder);
        }

        let share_path = self.data_home.join("hestia").join(name.as_str());
        if share_path.try_exists().map_err(|error| {
            ControllerError::operation(ControllerOperation::CreateLibrary, error)
        })? {
            return Err(ControllerError::LibraryAlreadyExists);
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
        library.save_config().map_err(|error| {
            ControllerError::operation(ControllerOperation::CreateLibrary, error)
        })?;

        self.activate(library).await
    }

    pub async fn initialize_workspace(&self) -> ControllerResult<()> {
        let (database_manager, file_operations, library_paths) = {
            let state = self.state.lock().await;
            let AppState::Ready { library, workspace } = &*state else {
                return Err(ControllerError::NoLibrarySelected);
            };
            let library_paths = library
                .library_config
                .as_ref()
                .map(|config| {
                    config
                        .library_paths
                        .iter()
                        .filter_map(|path| path.path.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (
                Arc::clone(&workspace.database_manager),
                Arc::clone(&workspace.file_operations),
                library_paths,
            )
        };

        Migrator::up(database_manager.get_connection().as_ref(), None)
            .await
            .context("database migration failed")
            .map_err(|error| {
                ControllerError::operation(ControllerOperation::InitializeLibrary, error)
            })?;
        if library_paths.is_empty() {
            return Err(ControllerError::NoContentFolders);
        }
        file_operations
            .upsert_root_folders(library_paths)
            .await
            .map_err(|error| {
                ControllerError::operation(ControllerOperation::InitializeLibrary, error)
            })?;
        Ok(())
    }

    pub async fn scan(&self) -> ControllerResult<ScanReport> {
        let (file_operations, library_paths) = {
            let state = self.state.lock().await;
            let AppState::Ready { library, workspace } = &*state else {
                return Err(ControllerError::NoLibrarySelected);
            };
            let paths = library
                .library_config
                .as_ref()
                .map(|config| {
                    config
                        .library_paths
                        .iter()
                        .filter_map(|path| path.path.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (Arc::clone(&workspace.file_operations), paths)
        };

        let scanner = DirectoryScanner::new(file_operations);
        let mut result = ScanReport {
            files_scanned: 0,
            changed: 0,
        };
        for path in library_paths {
            let report = scanner.sync_directory(&path).await.map_err(|error| {
                ControllerError::operation(ControllerOperation::ScanLibrary, error)
            })?;
            result.files_scanned += report.files_scanned;
            result.changed += report.files_inserted
                + report.files_updated
                + report.files_deleted
                + report.folders_inserted
                + report.folders_updated
                + report.folders_deleted;
        }
        Ok(result)
    }

    pub async fn generate_thumbnails(&self) -> ControllerResult<()> {
        let state = self.state.lock().await;
        let AppState::Ready { workspace, .. } = &*state else {
            return Err(ControllerError::NoLibrarySelected);
        };
        workspace
            .thumbnail_processor
            .queue_missing_files()
            .await
            .map_err(|error| {
                ControllerError::operation(ControllerOperation::GenerateThumbnails, error)
            })
    }

    pub async fn start_watching(&self) -> ControllerResult<mpsc::UnboundedReceiver<()>> {
        let (database_manager, paths) = {
            let mut state = self.state.lock().await;
            let AppState::Ready { library, workspace } = &mut *state else {
                return Err(ControllerError::NoLibrarySelected);
            };
            if workspace.watcher_started {
                return Err(ControllerError::operation(
                    ControllerOperation::StartWatcher,
                    anyhow::anyhow!("the library is already being watched"),
                ));
            }
            workspace.watcher_started = true;
            let paths = library
                .library_config
                .as_ref()
                .map(|config| {
                    config
                        .library_paths
                        .iter()
                        .filter_map(|path| path.path.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (Arc::clone(&workspace.database_manager), paths)
        };

        let (watcher_sender, watcher_receiver) = mpsc::unbounded_channel();
        let (changes_sender, changes_receiver) = mpsc::unbounded_channel();
        let event_handler = DatabaseFileWatcherEventHandler {
            db_operations: FileRepository::new(database_manager),
            changes: Some(changes_sender),
        };
        tokio::spawn(async move {
            if let Err(error) = FileWatcher::new(watcher_receiver)
                .run(Box::new(event_handler))
                .await
            {
                tracing::error!(%error, "File watcher stopped");
            }
        });
        for path in paths {
            watcher_sender
                .send(FileWatcherMessage::WatchPath(CanonPath::from(path)))
                .map_err(|error| {
                    ControllerError::operation(ControllerOperation::StartWatcher, error)
                })?;
        }
        Ok(changes_receiver)
    }

    pub async fn list_folders(&self) -> ControllerResult<Vec<FolderInfo>> {
        let database_manager = self.database_manager().await?;
        folders::Entity::find()
            .order_by_asc(folders::Column::Name)
            .all(database_manager.get_connection().as_ref())
            .await
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| ControllerError::operation(ControllerOperation::QueryLibrary, error))
    }

    pub async fn list_files(
        &self,
        folder_id: Option<i32>,
        search: &str,
    ) -> ControllerResult<Vec<FileInfo>> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let mut condition = Condition::all();
        if let Some(folder_id) = folder_id {
            let folder = folders::Entity::find_by_id(folder_id)
                .one(connection.as_ref())
                .await
                .map_err(|error| {
                    ControllerError::operation(ControllerOperation::QueryLibrary, error)
                })?
                .ok_or(ControllerError::FileNotFound)?;
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
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| ControllerError::operation(ControllerOperation::QueryLibrary, error))
    }

    pub async fn list_tags(&self) -> ControllerResult<Vec<TagInfo>> {
        let database_manager = self.database_manager().await?;
        tags::Entity::find()
            .order_by_asc(tags::Column::Name)
            .all(database_manager.get_connection().as_ref())
            .await
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| ControllerError::operation(ControllerOperation::QueryLibrary, error))
    }

    pub async fn create_tag(&self, name: &str) -> ControllerResult<()> {
        let name = Self::tag_name(name)?;
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        if tags::Entity::find()
            .filter(tags::Column::Name.eq(name))
            .one(connection.as_ref())
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?
            .is_some()
        {
            return Ok(());
        }
        tags::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            name: Set(name.to_string()),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        }
        .insert(connection.as_ref())
        .await
        .map(|_| ())
        .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))
    }

    pub async fn update_tag(&self, tag_id: i32, name: &str) -> ControllerResult<()> {
        let name = Self::tag_name(name)?;
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let tag = tags::Entity::find_by_id(tag_id)
            .one(connection.as_ref())
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?
            .ok_or(ControllerError::TagNotFound)?;
        let mut tag = tag.into_active_model();
        tag.name = Set(name.to_string());
        tag.updated_at = Set(chrono::Utc::now().naive_utc());
        tag.update(connection.as_ref())
            .await
            .map(|_| ())
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))
    }

    pub async fn delete_tag(&self, tag_id: i32) -> ControllerResult<()> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        let transaction = connection
            .begin()
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?;
        file_has_tags::Entity::delete_many()
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .exec(&transaction)
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?;
        tags::Entity::delete_by_id(tag_id)
            .exec(&transaction)
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?;
        transaction
            .commit()
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))
    }

    pub async fn assign_tag(&self, file_id: i32, tag_id: i32) -> ControllerResult<()> {
        let database_manager = self.database_manager().await?;
        let connection = database_manager.get_connection();
        if files::Entity::find_by_id(file_id)
            .one(connection.as_ref())
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?
            .is_none()
        {
            return Err(ControllerError::FileNotFound);
        }
        if tags::Entity::find_by_id(tag_id)
            .one(connection.as_ref())
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?
            .is_none()
        {
            return Err(ControllerError::TagNotFound);
        }
        if file_has_tags::Entity::find()
            .filter(file_has_tags::Column::FileId.eq(file_id))
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .one(connection.as_ref())
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))?
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
        .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))
    }

    pub async fn remove_tag(&self, file_id: i32, tag_id: i32) -> ControllerResult<()> {
        let database_manager = self.database_manager().await?;
        file_has_tags::Entity::delete_many()
            .filter(file_has_tags::Column::FileId.eq(file_id))
            .filter(file_has_tags::Column::TagId.eq(tag_id))
            .exec(database_manager.get_connection().as_ref())
            .await
            .map(|_| ())
            .map_err(|error| ControllerError::operation(ControllerOperation::ManageTags, error))
    }

    async fn database_manager(&self) -> ControllerResult<Arc<DatabaseManager>> {
        let state = self.state.lock().await;
        let AppState::Ready { workspace, .. } = &*state else {
            return Err(ControllerError::NoLibrarySelected);
        };
        Ok(Arc::clone(&workspace.database_manager))
    }

    fn tag_name(name: &str) -> ControllerResult<&str> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ControllerError::InvalidTagName);
        }
        Ok(name)
    }

    async fn activate(&self, library: Library) -> ControllerResult<LibraryInfo> {
        let info = LibraryInfo::from_path(
            library
                .share_path
                .clone()
                .ok_or(ControllerError::MissingStorageFolder)?,
        )?;
        let workspace = Workspace::open(&library)
            .await
            .map_err(|error| ControllerError::operation(ControllerOperation::OpenLibrary, error))?;
        *self.state.lock().await = AppState::Ready { library, workspace };
        Ok(info)
    }
}

impl Workspace {
    async fn open(library: &Library) -> Result<Self> {
        let database_path = library.get_canon_database_path()?;
        let connection_string = format!("sqlite:///{}", database_path.as_str()?);
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
            database_manager,
            file_operations,
            thumbnail_processor,
            watcher_started: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppController, ControllerError};
    use anyhow::Result;
    use tempfile::TempDir;

    #[tokio::test]
    async fn create_library_returns_an_owned_summary() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let library = controller.create_library("Photos", content.path()).await?;

        assert_eq!(library.name().as_str(), "Photos");
        assert_eq!(library.path(), data_home.path().join("hestia/Photos"));
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
        assert_eq!(libraries.first().unwrap().name().as_str(), "Photos");
        Ok(())
    }

    #[tokio::test]
    async fn select_library_opens_an_existing_library() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let creator = AppController::new_in(data_home.path())?;
        let created = creator.create_library("Photos", content.path()).await?;
        let controller = AppController::new_in(data_home.path())?;

        let selected = controller.select_library(created.path()).await?;

        assert_eq!(selected, created);
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
    async fn initialized_library_supports_the_qt_controller_api() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        std::fs::write(content.path().join("notes.txt"), "hello")?;
        let controller = AppController::new_in(data_home.path())?;
        controller.create_library("Notes", content.path()).await?;
        controller.initialize_workspace().await?;

        let report = controller.scan().await?;
        assert_eq!(report.files_scanned(), 1);
        let folders = controller.list_folders().await?;
        let files = controller
            .list_files(Some(folders[0].id()), "notes")
            .await?;
        assert_eq!(files.len(), 1);

        controller.create_tag("Important").await?;
        let tags = controller.list_tags().await?;
        assert_eq!(tags[0].name(), "Important");
        controller.assign_tag(files[0].id(), tags[0].id()).await?;
        controller.remove_tag(files[0].id(), tags[0].id()).await?;
        controller.delete_tag(tags[0].id()).await?;
        assert!(controller.list_tags().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn initialize_workspace_reports_when_no_library_is_selected() -> Result<()> {
        let data_home = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let error = controller
            .initialize_workspace()
            .await
            .expect_err("initialization should require a selected library");

        assert!(matches!(error, ControllerError::NoLibrarySelected));
        Ok(())
    }

    #[tokio::test]
    async fn create_library_rejects_path_traversal_names() -> Result<()> {
        let data_home = TempDir::new()?;
        let content = TempDir::new()?;
        let controller = AppController::new_in(data_home.path())?;

        let error = controller
            .create_library("../Photos", content.path())
            .await
            .expect_err("library names must not escape the library directory");

        assert!(matches!(error, ControllerError::InvalidLibraryName));
        Ok(())
    }
}
