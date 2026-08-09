use anyhow::{Error, Result, bail};
use chrono::Local;
use entity::thumbnails;
use sea_orm::{ActiveValue, Set};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThumbnailSize {
    Small,
    Medium,
    Large,
}

impl ThumbnailSize {
    #[must_use]
    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Small => (128, 128),
            Self::Medium => (256, 256),
            Self::Large => (512, 512),
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    #[must_use]
    pub const fn all() -> [ThumbnailSize; 3] {
        [Self::Small, Self::Medium, Self::Large]
    }

    #[must_use]
    pub const fn fallback() -> ThumbnailSize {
        Self::Medium
    }
}

impl fmt::Display for ThumbnailSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<ThumbnailSize> for String {
    fn from(size: ThumbnailSize) -> Self {
        size.as_str().to_string()
    }
}

impl TryFrom<&str> for ThumbnailSize {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "small" => Ok(ThumbnailSize::Small),
            "medium" => Ok(ThumbnailSize::Medium),
            "large" => Ok(ThumbnailSize::Large),
            _ => bail!("unsupported thumbnail size: {value}"),
        }
    }
}

impl TryFrom<String> for ThumbnailSize {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub size: ThumbnailSize,
    pub data: Vec<u8>,
    pub mime_type: String,
    pub file_size: usize,
}

impl Thumbnail {
    #[must_use]
    pub fn new(size: ThumbnailSize, data: Vec<u8>, mime_type: String) -> Self {
        let file_size = data.len();
        Self {
            size,
            data,
            mime_type,
            file_size,
        }
    }

    #[must_use]
    pub fn with_image_data(size: ThumbnailSize, data: Vec<u8>) -> Self {
        Self::new(size, data, "image/png".to_string())
    }

    #[must_use]
    pub fn size(&self) -> ThumbnailSize {
        self.size
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    #[must_use]
    pub fn file_size(&self) -> usize {
        self.file_size
    }

    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        self.size.dimensions()
    }

    /// Converts to `SeaORM` `ActiveModel` for database insertion
    #[must_use]
    pub fn to_active_model(self, file_id: i32) -> thumbnails::ActiveModel {
        let now = Local::now().naive_local();

        thumbnails::ActiveModel {
            id: ActiveValue::NotSet,
            file_id: Set(file_id),
            size: Set(self.size.to_string()),
            data: Set(self.data),
            mime_type: Set(self.mime_type),
            file_size: Set(self.file_size as i32),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }

    /// Creates thumbnail from `SeaORM` Model
    pub fn from_model(model: thumbnails::Model) -> Result<Self> {
        let size = ThumbnailSize::try_from(model.size)?;

        Ok(Self {
            size,
            data: model.data,
            mime_type: model.mime_type,
            file_size: model.file_size as usize,
        })
    }

    /// Returns true if this is an image thumbnail (not a file icon)
    #[must_use]
    pub fn is_image(&self) -> bool {
        self.mime_type == "image/png" || self.mime_type.starts_with("image/")
    }
}

impl From<thumbnails::Model> for Thumbnail {
    fn from(model: thumbnails::Model) -> Self {
        let size = match ThumbnailSize::try_from(model.size) {
            Ok(size) => size,
            Err(_) => ThumbnailSize::fallback(),
        };

        Self {
            size,
            data: model.data,
            mime_type: model.mime_type,
            file_size: model.file_size as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thumbnail_size_dimensions() {
        assert_eq!(ThumbnailSize::Small.dimensions(), (128, 128));
        assert_eq!(ThumbnailSize::Medium.dimensions(), (256, 256));
        assert_eq!(ThumbnailSize::Large.dimensions(), (512, 512));
    }

    #[test]
    fn test_thumbnail_size_as_str() {
        assert_eq!(ThumbnailSize::Small.as_str(), "small");
        assert_eq!(ThumbnailSize::Medium.as_str(), "medium");
        assert_eq!(ThumbnailSize::Large.as_str(), "large");
    }

    #[test]
    fn test_thumbnail_size_all() {
        let all_sizes = ThumbnailSize::all();
        assert_eq!(all_sizes.len(), 3);
        assert!(all_sizes.contains(&ThumbnailSize::Small));
        assert!(all_sizes.contains(&ThumbnailSize::Medium));
        assert!(all_sizes.contains(&ThumbnailSize::Large));
    }

    #[test]
    fn test_thumbnail_struct() {
        let data = vec![1, 2, 3, 4, 5];
        let mime_type = "image/png".to_string();
        let size = ThumbnailSize::Medium;

        let thumbnail = Thumbnail::new(size, data.clone(), mime_type.clone());

        assert_eq!(thumbnail.size(), size);
        assert_eq!(thumbnail.data(), &data[..]);
        assert_eq!(thumbnail.mime_type(), &mime_type);
        assert_eq!(thumbnail.file_size(), data.len());
        assert_eq!(thumbnail.dimensions(), (256, 256));
    }

    #[test]
    fn test_thumbnail_with_image_data() {
        let data = vec![10, 20, 30];
        let size = ThumbnailSize::Large;

        let thumbnail = Thumbnail::with_image_data(size, data.clone());

        assert_eq!(thumbnail.size(), size);
        assert_eq!(thumbnail.data(), &data[..]);
        assert_eq!(thumbnail.mime_type(), "image/png");
        assert_eq!(thumbnail.file_size(), data.len());
        assert_eq!(thumbnail.dimensions(), (512, 512));
    }

    #[test]
    fn test_thumbnail_seaorm_conversion() -> Result<()> {
        let original_data = vec![1, 2, 3, 4, 5];
        let size = ThumbnailSize::Small;
        let mime_type = "image/jpeg".to_string();
        let file_size = original_data.len();
        let file_id = 42;

        // Create thumbnail
        let thumbnail = Thumbnail::new(size, original_data.clone(), mime_type.clone());

        // Test to_active_model
        let active_model = thumbnail.clone().to_active_model(file_id);
        assert_eq!(active_model.file_id.clone().unwrap(), file_id);
        assert_eq!(active_model.size.clone().unwrap(), "small");
        assert_eq!(active_model.data.clone().unwrap(), original_data);
        assert_eq!(active_model.mime_type.clone().unwrap(), mime_type);
        assert_eq!(active_model.file_size.clone().unwrap(), file_size as i32);

        // Create a mock Model for testing from_model
        let now = Local::now().naive_local();
        let model = thumbnails::Model {
            id: 1,
            file_id,
            size: "small".to_string(),
            data: original_data.clone(),
            mime_type: mime_type.clone(),
            file_size: file_size as i32,
            created_at: now,
            updated_at: now,
        };

        // Test from_model
        let restored_thumbnail = Thumbnail::from_model(model)?;
        assert_eq!(restored_thumbnail.size(), size);
        assert_eq!(restored_thumbnail.data(), &original_data[..]);
        assert_eq!(restored_thumbnail.mime_type(), &mime_type);
        assert_eq!(restored_thumbnail.file_size(), file_size);
        Ok(())
    }

    #[test]
    fn test_thumbnail_size_conversions() -> Result<()> {
        assert_eq!(ThumbnailSize::try_from("small")?, ThumbnailSize::Small);
        assert_eq!(ThumbnailSize::try_from("medium")?, ThumbnailSize::Medium);
        assert_eq!(ThumbnailSize::try_from("large")?, ThumbnailSize::Large);

        let Err(error) = ThumbnailSize::try_from("invalid") else {
            bail!("an unsupported thumbnail size should fail");
        };
        assert_eq!(error.to_string(), "unsupported thumbnail size: invalid");

        assert_eq!(
            ThumbnailSize::try_from("small".to_string())?,
            ThumbnailSize::Small
        );
        assert!(ThumbnailSize::try_from("invalid".to_string()).is_err());

        let size_str: String = ThumbnailSize::Medium.into();
        assert_eq!(size_str, "medium");
        Ok(())
    }

    #[test]
    fn test_thumbnail_is_image() {
        let png_thumbnail = Thumbnail::with_image_data(ThumbnailSize::Small, vec![1, 2, 3]);
        assert!(png_thumbnail.is_image());

        let jpeg_thumbnail = Thumbnail::new(
            ThumbnailSize::Medium,
            vec![1, 2, 3],
            "image/jpeg".to_string(),
        );
        assert!(jpeg_thumbnail.is_image());

        let pdf_thumbnail = Thumbnail::new(
            ThumbnailSize::Large,
            vec![1, 2, 3],
            "application/pdf".to_string(),
        );
        assert!(!pdf_thumbnail.is_image());
    }
}
