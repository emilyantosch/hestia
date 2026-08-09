use anyhow::{Context, Result, ensure};
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use model::services::thumbnail::{Thumbnail, ThumbnailSize};
use std::io::Cursor;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub struct ThumbnailGenerator {
    filter_type: FilterType,
}

impl ThumbnailGenerator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            filter_type: FilterType::Lanczos3, // High quality resizing
        }
    }

    #[must_use]
    pub fn with_filter(filter_type: FilterType) -> Self {
        Self { filter_type }
    }

    pub fn generate_image_thumbnail(
        &self,
        image_data: &[u8],
        size: ThumbnailSize,
    ) -> Result<Thumbnail> {
        let img =
            image::load_from_memory(image_data).context("Failed to load image from memory")?;

        let (target_width, target_height) = size.dimensions();

        // `resize` preserves the aspect ratio while honoring the selected filter.
        let thumbnail = img.resize(target_width, target_height, self.filter_type);

        // Encode as PNG for consistent output
        let mut output = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
            .context("Failed to encode thumbnail")?;

        Ok(Thumbnail::with_image_data(size, output))
    }

    pub async fn generate_from_file_path(
        &self,
        file_path: &Path,
        size: ThumbnailSize,
    ) -> Result<Thumbnail> {
        // Check if file exists first
        ensure!(
            file_path.exists(),
            "file not found: {}",
            file_path.display()
        );

        // Detect file type first
        let file_data = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        let mime_type =
            infer::get(&file_data).map_or("application/octet-stream", |kind| kind.mime_type());

        match mime_type {
            mime if mime.starts_with("image/") => self.generate_image_thumbnail(&file_data, size),
            _ => Self::generate_file_icon(mime_type, size),
        }
    }

    fn generate_file_icon(mime_type: &str, size: ThumbnailSize) -> Result<Thumbnail> {
        let (width, height) = size.dimensions();
        let mut img = ImageBuffer::new(width, height);

        // Generate themed background based on file type
        let bg_color = Self::get_file_type_color(mime_type);

        // Fill background
        for pixel in img.pixels_mut() {
            *pixel = bg_color;
        }

        // TODO: Add file type icon/text overlay in future iteration
        let mut output = Vec::new();
        let dynamic_img = DynamicImage::ImageRgba8(img);
        dynamic_img
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
            .context("Failed to encode file icon")?;

        Ok(Thumbnail::new(size, output, "image/png".to_string()))
    }

    fn get_file_type_color(mime_type: &str) -> Rgba<u8> {
        match mime_type {
            mime if mime.starts_with("text/") => Rgba([74, 144, 226, 255]), // Blue
            mime if mime.starts_with("application/pdf") => Rgba([231, 76, 60, 255]), // Red
            mime if mime.starts_with("video/") => Rgba([155, 89, 182, 255]), // Purple
            mime if mime.starts_with("audio/") => Rgba([46, 204, 113, 255]), // Green
            _ => Rgba([149, 165, 166, 255]),                                // Gray for unknown
        }
    }
}

impl Default for ThumbnailGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thumbnail_generator_creation() {
        let generator = ThumbnailGenerator::new();
        assert_eq!(generator.filter_type, FilterType::Lanczos3);

        let generator_default = ThumbnailGenerator::default();
        assert_eq!(generator_default.filter_type, FilterType::Lanczos3);

        let generator_custom = ThumbnailGenerator::with_filter(FilterType::Nearest);
        assert_eq!(generator_custom.filter_type, FilterType::Nearest);
    }
}
