#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{Context as _, Result};
use gpui_kit::component::{ActiveTheme as _, text::TextView};
#[cfg(not(target_os = "linux"))]
use gpui_kit::{AppContext as _, Entity, component::WindowExt as _};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Task, TestSupportExt as _,
    Window, div, img, px,
};
#[cfg(not(target_os = "linux"))]
use gpui_wry::WebView;
use std::io::Read as _;
#[cfg(not(target_os = "linux"))]
use std::io::Seek as _;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "linux"))]
use wry::http::{Request, Response, StatusCode};

const TEXT_LIMIT: u64 = 1024 * 1024;
#[cfg(not(target_os = "linux"))]
const VIDEO_URL: &str = if cfg!(target_os = "windows") {
    "http://hestia-preview.localhost/"
} else {
    "hestia-preview://localhost/"
};
#[cfg(not(target_os = "linux"))]
const PLAYER: &str = include_str!("preview.html");

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Markdown,
    Text,
    Image,
    Video,
}

impl Kind {
    pub(crate) fn for_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "log" | "csv" | "json" | "toml" | "yaml" | "yml" | "rs" => Some(Self::Text),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" => Some(Self::Image),
            "mp4" | "m4v" | "mov" | "webm" | "ogv" => Some(Self::Video),
            _ => None,
        }
    }
}

enum Content {
    Loading,
    Markdown(String),
    Text(String),
    Image(PathBuf),
    #[cfg(not(target_os = "linux"))]
    Video(Entity<WebView>),
    #[cfg(target_os = "linux")]
    ExternalVideo,
    Error(String),
}

pub(crate) struct Preview {
    content: Content,
    _load_task: Task<()>,
    #[cfg(not(target_os = "linux"))]
    tasks: Vec<Task<()>>,
}

impl Preview {
    pub(crate) fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(not(target_os = "linux"))]
        cx.on_release(|this, cx| {
            if let Content::Video(video) = &this.content {
                video.update(cx, |video, _| {
                    let _closed_player =
                        video.evaluate_script("document.querySelector('video')?.pause()");
                    video.hide();
                });
            }
        })
        .detach();
        let task = cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { load(&path).map(|loaded| (path, loaded)) })
                .await;
            let _closed_preview = view.update_in(cx, |this, window, cx| {
                #[cfg(target_os = "linux")]
                let _ = window;
                this.content = match result {
                    #[cfg(target_os = "linux")]
                    Ok((path, (Kind::Video, _))) => {
                        // ponytail: external playback on Linux; bundle libmpv when embedded playback is needed.
                        cx.open_with_system(&path);
                        Content::ExternalVideo
                    }
                    #[cfg(not(target_os = "linux"))]
                    Ok((path, (Kind::Video, _))) => match this.video(path, window, cx) {
                        Ok(video) => Content::Video(video),
                        Err(error) => Content::Error(format!("Could not open video: {error:#}")),
                    },
                    Ok((path, (Kind::Image, _))) => Content::Image(path),
                    Ok((_, (Kind::Markdown, text))) => Content::Markdown(text),
                    Ok((_, (Kind::Text, text))) => Content::Text(text),
                    Err(error) => Content::Error(format!("Could not preview file: {error:#}")),
                };
                cx.notify();
            });
        });
        Self {
            content: Content::Loading,
            _load_task: task,
            #[cfg(not(target_os = "linux"))]
            tasks: Vec::new(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn video(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<WebView>> {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let webview = wry::WebViewBuilder::new()
            .with_url(VIDEO_URL)
            .with_navigation_handler(|url| url == VIDEO_URL)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_ipc_handler(move |request| {
                if request.body() == "close" {
                    let _closed_preview = sender.send(());
                }
            })
            .with_asynchronous_custom_protocol(
                "hestia-preview".into(),
                move |_, request, responder| {
                    let path = path.clone();
                    std::thread::spawn(move || responder.respond(media_response(&path, &request)));
                },
            )
            .build_as_child(&*window)?;
        self.tasks.push(cx.spawn_in(window, async move |_, cx| {
            if receiver.recv().await.is_some() {
                let _closed_window = cx.update(Window::close_dialog);
            }
        }));
        Ok(cx.new(|cx| WebView::new(webview, window, cx)))
    }
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        let id = match &self.content {
            Content::Loading => "preview-loading",
            Content::Markdown(_) => "preview-markdown",
            Content::Text(_) => "preview-text",
            Content::Image(_) => "preview-image",
            #[cfg(not(target_os = "linux"))]
            Content::Video(_) => "preview-video",
            #[cfg(target_os = "linux")]
            Content::ExternalVideo => "preview-video",
            Content::Error(_) => "preview-error",
        };
        div()
            .id(id)
            .test_support()
            .size_full()
            .child(match &self.content {
                Content::Loading => div().child("Loading preview…").into_any_element(),
                Content::Markdown(text) => TextView::markdown("markdown-preview", text.clone())
                    .size_full()
                    .into_any_element(),
                Content::Text(text) => div()
                    .id("text-preview")
                    .size_full()
                    .overflow_y_scroll()
                    .font_family("monospace")
                    .whitespace_normal()
                    .child(text.clone())
                    .into_any_element(),
                Content::Image(path) => img(path.clone())
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Contain)
                    .with_fallback(|| {
                        div()
                            .child("Could not decode this image.")
                            .into_any_element()
                    })
                    .into_any_element(),
                #[cfg(not(target_os = "linux"))]
                Content::Video(video) => div().size_full().child(video.clone()).into_any_element(),
                #[cfg(target_os = "linux")]
                Content::ExternalVideo => div()
                    .p_4()
                    .child("Requested playback in your default video player. If nothing opens, check that xdg-open and a video player are installed. Closing this preview does not stop playback.")
                    .into_any_element(),
                Content::Error(error) => div()
                    .p_4()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
                    .into_any_element(),
            })
            .min_h(px(0.))
    }
}

fn load(path: &Path) -> Result<(Kind, String)> {
    let kind = Kind::for_path(path).context("This file type is not supported")?;
    let file = std::fs::File::open(path).context("The file could not be read")?;
    anyhow::ensure!(file.metadata()?.is_file(), "Not a regular file");
    let mut text = String::new();
    if matches!(kind, Kind::Markdown | Kind::Text) {
        file.take(TEXT_LIMIT + 1)
            .read_to_string(&mut text)
            .context("Text must be UTF-8")?;
        anyhow::ensure!(
            text.len() as u64 <= TEXT_LIMIT,
            "Text preview is limited to 1 MiB"
        );
    }
    Ok((kind, text))
}

#[cfg(not(target_os = "linux"))]
fn media_response(path: &Path, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    match serve_media(path, request) {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "Video preview request failed");
            let mut response = Response::new(b"Could not read video".to_vec());
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn serve_media(path: &Path, request: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>> {
    let response = Response::builder().header("Access-Control-Allow-Origin", "*");
    if request.method() != "GET" && request.method() != "HEAD" {
        return Ok(response.status(405).body(Vec::new())?);
    }
    match request.uri().path() {
        "/" => {
            return Ok(response
                .header("Content-Type", "text/html; charset=utf-8")
                .body(PLAYER.as_bytes().to_vec())?);
        }
        "/video" => {}
        _ => return Ok(response.status(404).body(Vec::new())?),
    }
    // The URL never selects a filesystem path: only the selected file is exposed.
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let range = request
        .headers()
        .get("Range")
        .and_then(|value| value.to_str().ok());
    let Some((start, end)) = byte_range(range, len) else {
        return Ok(response
            .status(416)
            .header("Content-Range", format!("bytes */{len}"))
            .body(Vec::new())?);
    };
    let mime = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        _ => "video/mp4",
    };
    let mut response = response
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes");
    if range.is_some() {
        response = response
            .status(206)
            .header("Content-Range", format!("bytes {start}-{end}/{len}"));
    }
    let size = end - start + 1;
    // ponytail: Wry buffers responses; range requests use 8 MiB chunks. Non-range clients
    // are limited to 64 MiB; use a streaming transport if such clients need larger videos.
    if range.is_none() && size > 64 * 1024 * 1024 {
        bail!("Video client must support byte ranges for files larger than 64 MiB");
    }
    let mut body = Vec::new();
    if request.method() != "HEAD" {
        file.seek(std::io::SeekFrom::Start(start))?;
        file.take(size).read_to_end(&mut body)?;
    }
    Ok(response.header("Content-Length", size).body(body)?)
}

#[cfg(not(target_os = "linux"))]
fn byte_range(range: Option<&str>, len: u64) -> Option<(u64, u64)> {
    let last = len.checked_sub(1)?;
    let Some(range) = range else {
        return Some((0, last));
    };
    let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        (len.saturating_sub(suffix), last)
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            last
        } else {
            end.parse::<u64>().ok()?.min(last)
        };
        (start, end)
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end.min(start.saturating_add(8 * 1024 * 1024 - 1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_loading() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let text = directory.path().join("notes.MD");
        std::fs::write(&text, "# Heading\n\nFull text, not an excerpt.")?;
        assert_eq!(
            load(&text)?,
            (
                Kind::Markdown,
                "# Heading\n\nFull text, not an excerpt.".into()
            )
        );
        std::fs::write(&text, [0xff])?;
        assert!(load(&text).is_err());
        std::fs::write(&text, vec![b'x'; TEXT_LIMIT as usize + 1])?;
        assert!(load(&text).is_err());
        assert!(load(&directory.path().join("missing.txt")).is_err());
        assert_eq!(Kind::for_path(Path::new("photo.JPEG")), Some(Kind::Image));
        assert_eq!(Kind::for_path(Path::new("clip.MOV")), Some(Kind::Video));
        assert_eq!(Kind::for_path(Path::new("app.exe")), None);
        let video = directory.path().join("clip.mp4");
        std::fs::write(&video, b"0123456789")?;
        assert_eq!(load(&video)?, (Kind::Video, String::new()));
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn video_ranges() -> Result<()> {
        let directory = tempfile::tempdir()?;
        assert_eq!(byte_range(Some("bytes=2-4"), 10), Some((2, 4)));
        assert_eq!(byte_range(Some("bytes=5-"), 10), Some((5, 9)));
        assert_eq!(byte_range(Some("bytes=-3"), 10), Some((7, 9)));
        assert_eq!(byte_range(Some("bytes=8-99"), 10), Some((8, 9)));
        assert_eq!(
            byte_range(Some("bytes=0-"), u64::MAX),
            Some((0, 8 * 1024 * 1024 - 1))
        );
        for range in [
            "bytes=10-",
            "bytes=5-2",
            "bytes=-0",
            "bytes=0-1,3-4",
            "nonsense",
        ] {
            assert_eq!(byte_range(Some(range), 10), None);
        }
        assert_eq!(byte_range(None, 0), None);
        let video = directory.path().join("clip.mp4");
        std::fs::write(&video, b"0123456789")?;
        let request = Request::builder()
            .uri("hestia-preview://localhost/video")
            .header("Range", "bytes=2-4")
            .body(Vec::new())?;
        let response = serve_media(&video, &request)?;
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["Content-Range"], "bytes 2-4/10");
        assert_eq!(response.body(), b"234");
        let request = Request::builder()
            .uri("hestia-preview://localhost/etc/passwd")
            .body(Vec::new())?;
        assert_eq!(serve_media(&video, &request)?.status(), 404);
        Ok(())
    }
}
