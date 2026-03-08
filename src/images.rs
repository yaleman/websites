use std::io::Cursor;

use image::{GenericImageView, ImageFormat};

use crate::constants::THUMBNAIL_MAX_SIZE;

pub(crate) struct ThumbnailResult {
    pub(crate) bytes: Vec<u8>,
    pub(crate) extension: String,
    pub(crate) mime_type: String,
    pub(crate) byte_length: i32,
    pub(crate) width: Option<i32>,
    pub(crate) height: Option<i32>,
}

pub(crate) async fn generate_thumbnail(
    bytes: Vec<u8>,
    extension: &str,
) -> Result<(Option<(u32, u32)>, Option<ThumbnailResult>), String> {
    let extension = extension.to_string();
    tokio::task::spawn_blocking(move || {
        let format = ImageFormat::from_extension(&extension);
        let image = match format {
            Some(format) => image::load_from_memory_with_format(&bytes, format),
            None => image::load_from_memory(&bytes),
        };

        let Ok(image) = image else {
            return Ok((None, None));
        };

        let (width, height) = image.dimensions();
        let thumbnail = image.thumbnail(THUMBNAIL_MAX_SIZE, THUMBNAIL_MAX_SIZE);
        let thumb_format = format.unwrap_or(ImageFormat::Png);
        let thumb_ext = image_format_extension(thumb_format).to_string();
        let mut thumb_bytes = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut thumb_bytes), thumb_format)
            .map_err(|error| error.to_string())?;

        let mime_type = mime_from_extension(thumb_ext.as_str()).to_string();
        let byte_length = i32::try_from(thumb_bytes.len()).unwrap_or(i32::MAX);
        let width_i32 = i32::try_from(thumbnail.width()).ok();
        let height_i32 = i32::try_from(thumbnail.height()).ok();

        Ok((
            Some((width, height)),
            Some(ThumbnailResult {
                bytes: thumb_bytes,
                extension: thumb_ext,
                mime_type,
                byte_length,
                width: width_i32,
                height: height_i32,
            }),
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn image_format_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        _ => "png",
    }
}

pub(crate) fn mime_from_extension(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
