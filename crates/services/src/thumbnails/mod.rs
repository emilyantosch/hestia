pub mod generator;
#[expect(
    clippy::module_inception,
    reason = "the inner module contains the thumbnail processor API"
)]
pub mod thumbnails;
