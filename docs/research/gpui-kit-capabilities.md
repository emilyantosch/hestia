# What can GPUI Kit give the pre-alpha UI?

Research for issue #40 (child of map #38). Researched 2026-10-28 against the versions pinned in `Cargo.lock` at `66d2b13`:

| Crate (Cargo.lock) | Version | Role |
| --- | --- | --- |
| `gpui-kit` | 0.6.4 | Facade. `use gpui_kit::*` re-exports GPUI. `component`, `base` and `assets` are its layers. |
| `gpui-component` / `gpui-base` | 0.6.4 | Styled components (List, Tree, Table) and their behaviour layer |
| `gpui-pre` (+ `-linux`, `-macos`, `-scheduler`, …) | 0.3.5 | GPUI as published by gpui-kit, cut from Zed rev `d89e9c2124b2786a390c7a451c7488601b4da2e1` (see `gpui-pre-0.3.5/Cargo.toml`) |

Sources are the crate sources in `~/.cargo/registry/src/index.crates.io-*/`. The exception is `gpui-pre-macos-0.3.5`, which was downloaded from crates.io; its sha256 matches the Cargo.lock checksum. Paths below are relative to those crate roots. Paths starting `crates/` are in this repo.

## Summary

| Need | Available | Gap / work for Hestia |
| --- | --- | --- |
| Folder picker | `cx.prompt_for_paths(PathPromptOptions { directories: true, .. })`: NSOpenPanel on macOS, xdg-desktop-portal on Linux. Tests can drive it. | Linux needs a FileChooser portal at runtime, or the call returns an error |
| 10k+ lists, grids and trees | `uniform_list`, `list(ListState)`, `v_virtual_list`, kit `List`/`Tree`/`Table` | No virtualized **grid**: build one from `uniform_list` rows of N tiles. The current app turns virtualization off. |
| Thumbnails | `img()` decodes on the background executor, with `with_loading`/`with_fallback`, `ImageCache` trait and `ImageSource::Render/Image` | No downscaling and no eviction. The global cache holds every decoded image until it is removed. |
| Tokio work → views | Nothing tokio-specific. Use GPUI `cx.spawn` / `spawn_in` + `WeakEntity::update`. | Hestia owns its runtime (already does). Awaiting tokio handles directly is fine in production. Tests need `allow_parking` or polling. |
| Channel → live views | Pattern used by gpui-component itself: a `cx.spawn` loop over `rx.recv().await` → `cx.update(..)` | Wire `start_watching()`'s receiver the same way, with debouncing and incremental model updates |

## 1. Native folder picker (macOS + Linux)

**What exists**
- `App::prompt_for_paths(options: PathPromptOptions) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>>` (`gpui-pre-0.3.5/src/app.rs:1618-1628`). `None` means the user cancelled. `Err` happens only on Linux, when the picker cannot open.
- `PathPromptOptions { files, directories, multiple, prompt: Option<SharedString> }` (`gpui-pre-0.3.5/src/platform.rs:2355-2364`).
- macOS: `NSOpenPanel` with `setCanChooseDirectories`, `setCanChooseFiles`, `setAllowsMultipleSelection`, `setCanCreateDirectories(true)`, `setResolvesAliases(false)`, and `beginWithCompletionHandler` (`gpui-pre-macos-0.3.5/src/platform.rs:816-862`). The panel is app-modal, not a window sheet. Only file URLs are returned.
- Linux (Wayland and X11, both on by default): `ashpd::desktop::file_chooser::OpenFileRequest` with `.directory(options.directories)`, `.multiple(..)`, `.modal(true)` and the title "Open Folder" (`gpui-pre-linux-0.3.5/src/linux/platform.rs:456-513`). Returned URIs are converted with `Url::to_file_path`.
- Tests: `TestAppContext::simulate_path_prompt_response(|opts| Some(vec![..]))` and `did_prompt_for_paths()` (`gpui-pre-0.3.5/src/app/test_context.rs:355-367`). The test platform queues the prompt (`platform/test/platform.rs:521-530`), so the add-root flow can be tested headlessly.
- `App::reveal_path` and `open_with_system` sit next to it (`app.rs:1643-1650`) and are useful for "Show in Finder/Files".

**Caveats**
- Linux needs a running xdg-desktop-portal with a FileChooser backend (gtk, gnome, kde, …). Without one, the result is `Err("Couldn't open file picker due to missing xdg-desktop-portal implementation.")` (`linux/platform.rs:56-57, 490`). Minimal tiling-WM setups may lack it, so show the error rather than ignore it.
- On Linux the `files` flag is ignored: only `directory` is passed. This doesn't matter for picking roots.
- Under Flatpak sandboxing, the portal would return document-portal paths, not the real paths. That is not relevant to the pre-alpha target.

**Implication:** Use `cx.prompt_for_paths` with `directories: true, files: false`, map the result into `AppController::create_library` on tokio, and show the portal error to the user on Linux.

## 2. Virtualized lists, grids and trees at 10k+ entries

**What exists**
- `gpui::uniform_list(id, item_count, |range, window, cx| Vec<R>)` renders only the visible range of fixed-height rows (`gpui-pre-0.3.5/src/elements/uniform_list.rs:18-30`). `UniformListScrollHandle` provides `scroll_to_item`, `logical_scroll_top_index` and `track_scroll` (`:136-252, 683`). It needs a parent with a fixed or max height.
- `gpui::list(ListState, render_item)` handles variable-height rows backed by a `SumTree`. `ListState::splice(old_range, count)` makes incremental inserts and removals cheap, and `reset`, `scroll_to_reveal_item` and `remeasure` are also there (`elements/list.rs:24, 314-503, 677`).
- `gpui_kit::component::{v_virtual_list, h_virtual_list, VirtualListScrollHandle}`: per-item `Rc<Vec<Size<Pixels>>>` and only the visible range is rendered (`gpui-base-0.6.4/src/virtual_list.rs:1-12, 139-210`, re-exported by `gpui-component-0.6.4/src/virtual_list.rs`).
- Kit `List` + `ListDelegate` (`items_count`, `render_item`, `perform_search`, `load_more`, `load_more_threshold`) is built on `v_virtual_list` (`gpui-component-0.6.4/src/list/list.rs:552`, `list/delegate.rs`).
- Kit `Tree` / `TreeState`: `TreeItem`s are flattened into `entries` (expanded nodes only) and rendered with `uniform_list` (`gpui-base-0.6.4/src/tree.rs:284-318, 423`). It emits `TreeEvent::Expanded/Collapsed(id)` (`:91-96`).
- Kit `Table` virtualizes rows with `uniform_list` and columns with `virtual_list` (`gpui-component-0.6.4/src/table/state.rs:2088, 2440`). It could serve as a list/details view.

**Caveats / gaps**
- **There is no virtualized grid** in gpui, gpui-base or gpui-component. `div().grid()` lays out every child. The standard approach is a `uniform_list` over `ceil(n / columns)` rows, where each row is an `h_flex` of up to `columns` tiles. Column count comes from the measured width, which `ui.rs` already computes.
- `TreeState` takes the **entire** `TreeItem` hierarchy up front, and each expand or collapse re-flattens it. That is O(visible) and fine for thousands of folders. A node counts as a folder only if `children` is non-empty (`tree.rs:132-134`), so empty folders and lazily-loaded folders show as leaves. `set_items` clears the selection (`:214-219`). Expansion state lives in each `TreeItem`'s `Rc<RefCell<..>>`, so a rebuild from fresh `FolderInfo`s loses expansion unless Hestia re-applies `.expanded(true)`.
- **The current app turns virtualization off.** In `crates/app/src/ui.rs`:
  - The folder tree's container is sized to `rows * ROW_HEIGHT` (`folders()`, ~l.917-946), so the `uniform_list` viewport equals the full content and every row renders. Per-folder counts are also computed O(folders × files) on every render.
  - The file grid is `div().grid().grid_cols(columns).children(all tiles)` inside an `overflow_y_scroll` div (~l.1555-1566). That builds every tile, each with an `img()`, on every frame.

**Implication:** Replace the grid with a `uniform_list` of tile rows and let the folder tree fill a flex-height container so its `uniform_list` actually virtualizes. Keep `TreeState` for folders, but preserve expansion and selection across rebuilds. Precompute folder counts outside `render`.

## 3. Async image loading and caching for thumbnails

**What exists**
- `img(source)` takes `ImageSource::{Resource(Path|Uri|Embedded), Render(Arc<RenderImage>), Image(Arc<Image>), Custom(fn)}` (`gpui-pre-0.3.5/src/elements/img.rs:42-51, 201`).
- Loading goes through the asset system. `App::asset_entry` → `CachedLoad::new`, which runs `Asset::load` on the **background executor** (`asset_cache.rs:26-28`). `ImageAssetLoader` reads the file and decodes it with `image` (GIF/WebP frames, otherwise `decode_static_image`; `img.rs:619-735`). Supported formats are listed in `Img::extensions()` (`img.rs:213-219`).
- `StyledImage::with_loading(..)` / `with_fallback(..)`, with `LOADING_DELAY` = 200 ms before the loading placeholder appears (`img.rs:32, 166-173`).
- Caches: by default, the **global** asset cache. Entries are "cached until `remove_asset`" (`app.rs:2700-2708`; `ImageSource::remove_asset` at `img.rs:576`). Alternatives:
  - `image_cache(provider)` element / `Img::image_cache(&Entity<I>)` with a pluggable `ImageCache` trait (`elements/image_cache.rs:14, 197-219`, `img.rs:221-235`).
  - `RetainAllImageCache` / `retain_all(id)`, which is a plain `HashMap` that never evicts. Its doc comment says "LRU", but the code is a HashMap (`image_cache.rs:221-307`).
- `ImageSource::Image(Arc<Image::from_bytes(format, bytes)>)` decodes in-memory bytes through the asset system (`img.rs:569`, `platform.rs:2774-2823`). That fits Hestia's thumbnails, which are DB blobs.
- `App::drop_image(Arc<RenderImage>, ..)` frees a texture from the GPU atlas in every window (`app.rs:2782`, `window.rs:4894`).

**Caveats / gaps**
- **No downscaling.** `img(path)` decodes the full-resolution original. With 10k photos and the default global cache, every image ever shown stays decoded in RAM and in the GPU atlas. Neither the global cache nor `RetainAllImageCache` evicts.
- No built-in LRU or size-bounded cache. A bounded `ImageCache` impl must call `remove_asset` / `drop_image` itself (the trait doc requires this, `image_cache.rs:195-196`).
- In Hestia, `FileInfo::thumbnail_path()` currently says thumbnails are DB blobs with no frontend adapter (`crates/controllers/src/lib.rs:152-153`). `generate_thumbnails()` exists (`:530`) but nothing exposes the bytes to the UI. The grid's tile calls `img(file.info.path())` on the original (`ui.rs` ~l.1880).

**Implication:** Serve small pre-generated thumbnails from the backend as bytes (or files), feed them as `ImageSource::Image`, or as `Resource::Path` to a thumbnail cache file. Wrap the grid in a custom bounded `ImageCache` (LRU that calls `remove_asset`/`drop_image`). Use `with_loading`/`with_fallback` for placeholders. Never point `img()` at originals in the grid.

## 4. Running tokio backend work and returning results to views

**What exists**
- GPUI runs its own executors (`BackgroundExecutor::spawn` for `Send` futures, `ForegroundExecutor` on the main thread; `gpui-pre-0.3.5/src/executor.rs:68-115, 290-354`). Entities expose `Context::spawn(async move |weak, cx| ..)` and `spawn_in(window, async move |weak, cx| ..)` (`app/context.rs:238, 696`). These return a `Task` that must be held or `.detach()`ed, and dropping the `Task` cancels the work. Results come back through `WeakEntity::update` / `update_in`, then `cx.notify()`.
- **No tokio integration ships in this crate family.** No `gpui_tokio` equivalent exists among the `gpui-pre-*`/`gpui-kit` crates in Cargo.lock. The only tokio user is `gpui-pre-reqwest`, a reqwest fork.
- Hestia already does this. `main.rs` builds a `tokio::runtime::Runtime` before `application().run(..)` and keeps it alive for the process. `demo.rs` stores `Handle::current()` and returns `tokio::task::JoinHandle`s from `runtime.spawn(..)` (`add_tag`, `create_tag`, …). `ui.rs::save_tag_change` (~l.522-556) awaits them in `cx.spawn_in`, polling `is_finished()` on a 5 ms GPUI timer. The reason, per the code comment, is that a wake from a tokio worker is rejected by GPUI's deterministic test scheduler.

**Caveats**
- The polling exists only for tests. In production, awaiting a `JoinHandle` (or a `tokio::sync::oneshot`) from a GPUI foreground task works, because GPUI's wakers are thread-safe. The test scheduler panics on non-deterministic wakes unless parking is allowed: `TestAppContext::executor().allow_parking()` (`executor.rs:220-233`). The scheduler test `test_nondeterministic_wake_allowed_with_parking` shows that foreign-thread wakes are then accepted (`gpui-pre-scheduler-0.3.5/src/tests.rs:590-633`). The trade-off is that the test loses determinism.
- Do not `block_on` tokio from the GPUI main thread except at start-up, as `main.rs` does now. It freezes the UI.
- `App` drops tasks before globals, so a long-lived tokio runtime should be owned outside GPUI globals or outlive `run` (`app.rs:730-734` comment).

**Implication:** Keep one Hestia-owned tokio runtime and a small helper such as `fn run<T>(&self, fut) -> impl Future<Output = Result<T>>`, used from `cx.spawn_in`. Either keep the polling helper, or switch tests to `allow_parking()` and await the handle directly. Pick one and use it everywhere.

## 5. Pushing channel notifications into views (live FS changes)

**What exists**
- The pattern used inside gpui-component itself: `ThemeRegistry::_watch_themes_dir` forwards `notify` events into a `smol::channel`. It then runs `cx.spawn(async move |cx| while rx.recv().await.is_ok() { cx.update(Self::reload_themes); })` (`gpui-component-0.6.4/src/theme/registry.rs:185-222`). The same shape works with `tokio::sync::mpsc::UnboundedReceiver::recv()`, which needs no tokio runtime context to poll.
- `gpui_kit::base::async_util::{unbounded, Sender, Receiver}` re-exports smol channels (`gpui-base-0.6.4/src/async_util.rs`). Entities can fan out via `EventEmitter` + `cx.subscribe`, or with `cx.observe` + `cx.notify()`. Hestia already uses both for the tree, slider and inputs (`ui.rs` ~l.220-290).
- Hestia's backend: `AppController::start_watching() -> mpsc::UnboundedReceiver<()>` (`crates/controllers/src/lib.rs:543`). It is a bare "something changed" ping, with no payload.

**Caveats**
- Same test-determinism issue as in section 4: a tokio-side `send` wakes the GPUI task from a foreign thread.
- `()` pings come in bursts during bulk FS operations. Coalesce them: drain with `try_recv` and/or wait ~100-250 ms on `cx.background_executor().timer(..)` before re-querying. Otherwise each ping triggers a full `list_files`/`list_folders` reload and rebuilds `TreeState` and the grid model.
- Holding a `WeakEntity` in the loop ends it when the view is dropped (`update` returns `Err`). Store the `Task` in the view, or `.detach()` it.

**Implication:** In the view constructor, call `start_watching()` on tokio and then run a `cx.spawn` loop: `rx.recv().await` → debounce → re-query on tokio → `this.update(cx, |v, cx| { v.apply(snapshot); cx.notify() })`. Payload-carrying change events from the backend would allow incremental `ListState::splice`/tree patches instead of full reloads.
