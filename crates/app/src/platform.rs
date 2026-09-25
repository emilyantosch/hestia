use gpui_kit::{App, KeyBinding, actions};

actions!(hestia, [OpenSearch, OpenSettings, OpenPreview]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-p", OpenSearch, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("space", OpenPreview, Some("FileManager && !Input")),
    ]);
    #[cfg(target_os = "macos")]
    macos::init(cx);
}

pub(crate) fn update_icon(dark: bool) {
    #[cfg(target_os = "macos")]
    macos::update_icon(dark);
    #[cfg(not(target_os = "macos"))]
    let _ = dark;
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{App, KeyBinding, OpenSearch, OpenSettings, actions};
    use block2::RcBlock;
    use gpui_kit::component::input;
    use gpui_kit::{Menu, MenuItem, OsAction, SystemMenuType};
    use objc2::{MainThreadMarker, runtime::Bool};
    use objc2_app_kit::{NSApplication, NSBezierPath, NSColor, NSImage};
    use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};

    actions!(hestia, [Quit, Hide, HideOthers, ShowAll, Minimize, Zoom]);

    pub(super) fn init(cx: &mut App) {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.on_action(|_: &Minimize, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _closed_window = window.update(cx, |_, window, _| window.minimize_window());
                });
            }
        });
        cx.on_action(|_: &Zoom, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _closed_window = window.update(cx, |_, window, _| window.zoom_window());
                });
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ]);
        cx.set_menus([
            Menu::new("Hestia").items([
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Hestia", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Hestia", Quit),
            ]),
            Menu::new("File").items([MenuItem::action("Search…", OpenSearch)]),
            Menu::new("Edit").items([
                MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            ]),
            Menu::new("Window").items([
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ]),
        ]);
    }

    pub(super) fn update_icon(dark: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::error!("Dock icon must be updated on the main thread");
            return;
        };
        let bytes: &[u8] = if dark {
            include_bytes!("../icons/hestia-dark.png")
        } else {
            include_bytes!("../icons/hestia-light.png")
        };
        let data = NSData::with_bytes(bytes);
        let Some(image) = NSImage::initWithData(mtm.alloc(), &data) else {
            tracing::error!("Could not decode the Hestia Dock icon");
            return;
        };
        let artwork = artwork_rect(image.size());
        let drawing = RcBlock::new(move |_: NSRect| {
            let (r, g, b) = if dark {
                (23., 26., 21.)
            } else {
                (243., 244., 245.)
            };
            NSColor::colorWithSRGBRed_green_blue_alpha(r / 255., g / 255., b / 255., 1.).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                NSRect::new(NSPoint::new(64., 64.), NSSize::new(896., 896.)),
                200.,
                200.,
            )
            .fill();
            image.drawInRect(artwork);
            Bool::YES
        });
        let image = NSImage::imageWithSize_flipped_drawingHandler(
            NSSize::new(1024., 1024.),
            false,
            &drawing,
        );
        // SAFETY: AppKit requires a non-nil image and the main thread; both are checked above.
        #[expect(
            unsafe_code,
            reason = "GPUI has no Dock icon API; AppKit's setter requires a non-nil image"
        )]
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }

    fn artwork_rect(source: NSSize) -> NSRect {
        let scale = 768. / source.width.max(source.height);
        let size = NSSize::new(source.width * scale, source.height * scale);
        NSRect::new(
            NSPoint::new((1024. - size.width) / 2., (1024. - size.height) / 2.),
            size,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dock_artwork_is_centered_padded_and_not_stretched() {
            for source in [
                NSSize::new(1860., 1127.),
                NSSize::new(1127., 1860.),
                NSSize::new(1024., 1024.),
            ] {
                let rect = artwork_rect(source);
                assert!(
                    (rect.size.width / rect.size.height - source.width / source.height).abs()
                        < 1e-9
                );
                assert!((rect.origin.x + rect.size.width / 2. - 512.).abs() < 1e-9);
                assert!((rect.origin.y + rect.size.height / 2. - 512.).abs() < 1e-9);
                assert!(rect.origin.x >= 128. && rect.origin.y >= 128.);
                assert!((rect.size.width.max(rect.size.height) - 768.).abs() < 1e-9);
            }
        }
    }
}
