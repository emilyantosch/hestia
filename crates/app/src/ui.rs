use crate::demo::{DemoFile, DemoLibrary};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IndexPath, Root, Selectable as _, Sizable as _,
    StyledExt as _, Theme, ThemeColor, ThemeMode, WindowExt as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::{List, ListDelegate, ListItem, ListState},
    oklch,
    sidebar::{SidebarGroup, SidebarItem as _, SidebarMenu, SidebarMenuItem},
    tag::Tag,
    tree::{Tree, TreeItem, TreeState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Task, WeakEntity, Window, div, img, prelude::FluentBuilder as _, px, rgb,
};

#[derive(Default)]
struct Appearance(Option<ThemeMode>);
impl gpui_kit::Global for Appearance {}

pub(crate) fn configure_theme(cx: &mut App) {
    if !cx.has_global::<Appearance>() {
        cx.set_global(Appearance::default());
    }
    apply_palette(Theme::global_mut(cx));
    Theme::sync_base(cx);
    crate::platform::update_icon(Theme::global(cx).is_dark());
}

fn apply_palette(theme: &mut Theme) {
    // Based on hestia_v2, with cooler neutrals in light mode.
    let (background, foreground, surface, border, muted, sunshine) = if theme.is_dark() {
        (
            0x0017_1a15,
            0x00f5_f0e7,
            0x0026_2c23,
            0x0049_5242,
            0x00b8_b2a7,
            0x0038_3a20,
        )
    } else {
        (
            0x00f3_f4f5,
            0x002c_2e30,
            0x00ff_ffff,
            0x00d5_d7da,
            0x0062_666b,
            0x00f5_e211,
        )
    };
    let background = rgb(background).into();
    let foreground = rgb(foreground).into();
    let surface = rgb(surface).into();
    let border = rgb(border).into();
    let muted = rgb(muted).into();
    let grass: Hsla = rgb(0x008e_d462).into();
    let ink = rgb(0x002c_2e2a).into();
    let hover = grass.opacity(0.15);
    let active = grass.opacity(0.25);
    theme.font_size = px(14.);
    theme.radius = px(8.);
    theme.colors = ThemeColor {
        background,
        foreground,
        border,
        muted: background,
        muted_foreground: muted,
        popover: surface,
        popover_foreground: foreground,
        primary: grass,
        primary_foreground: ink,
        primary_hover: grass.opacity(0.9),
        primary_active: grass.opacity(0.8),
        secondary: surface,
        secondary_foreground: foreground,
        accent: hover,
        accent_foreground: foreground,
        button: surface,
        button_foreground: foreground,
        button_hover: hover,
        button_active: active,
        button_primary: grass,
        button_primary_foreground: ink,
        button_primary_hover: grass.opacity(0.9),
        button_primary_active: grass.opacity(0.8),
        input: border,
        caret: foreground,
        ring: foreground,
        selection: active,
        sidebar: surface,
        sidebar_foreground: foreground,
        sidebar_border: border,
        sidebar_accent: grass,
        sidebar_accent_foreground: ink,
        scrollbar_thumb: border,
        scrollbar_thumb_hover: muted,
        info: rgb(0x002b_a0ff).into(),
        danger: rgb(0x00ff_705d).into(),
        warning: rgb(sunshine).into(),
        warning_foreground: foreground,
        ..theme.colors
    };
    theme.tokens = theme.colors.into();
}

pub(crate) struct FileManager {
    library: DemoLibrary,
    focus_handle: FocusHandle,
    query: String,
    active_tag: Option<i32>,
    active_folder: Option<i32>,
    folder_tree: Entity<TreeState>,
    _folder_subscription: Subscription,
    selected: Option<usize>,
    tag_input: Entity<InputState>,
    editing_tag: Option<i32>,
    creating_tag: bool,
    editing_color: Option<u32>,
    editing_sub_tags: Vec<i32>,
    color_input: Entity<InputState>,
    color_picker: Entity<ColorPickerState>,
    sub_tag_search: Entity<InputState>,
    confirming_tag_delete: bool,
    saving_tag: bool,
    tag_error: Option<String>,
    _tag_subscriptions: Vec<Subscription>,
    _appearance_subscription: Subscription,
}

impl FileManager {
    pub(crate) fn new(library: DemoLibrary, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe_window_appearance(window, |_, window, cx| {
            if cx.global::<Appearance>().0.is_none() {
                Theme::sync_system_appearance(Some(window), cx);
                configure_theme(cx);
                cx.notify();
            }
        });
        let view = cx.weak_entity();
        let handle = window.window_handle();
        App::on_action(cx, move |_: &crate::platform::OpenSearch, cx| {
            let view = view.clone();
            cx.defer(move |cx| {
                let _closed_window = handle.update(cx, |_, window, cx| {
                    let _dropped_view = view.update(cx, |this, cx| this.open_search(window, cx));
                });
            });
        });
        let view = cx.weak_entity();
        App::on_action(cx, move |_: &crate::platform::OpenSettings, cx| {
            let view = view.clone();
            cx.defer(move |cx| {
                let _closed_window = handle.update(cx, |_, window, cx| {
                    let _dropped_view = view.update(cx, |_, cx| Self::open_settings(window, cx));
                });
            });
        });
        let tag_input = cx.new(|cx| InputState::new(window, cx).placeholder("Tag name"));
        let tag_subscription =
            cx.subscribe_in(
                &tag_input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        this.submit_tag(window, cx);
                    }
                    InputEvent::Change => {
                        this.tag_error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        let color_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Hex or oklch(L C H)"));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let sub_tag_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search sub-tags…"));
        let color_subscription =
            cx.subscribe_in(&color_input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::Change) && !this.saving_tag {
                    if let Ok(color) = parse_tag_color(&input.read(cx).value()) {
                        this.editing_color = color;
                        this.sync_color_picker(window, cx);
                    }
                    cx.notify();
                }
            });
        let picker_subscription =
            cx.subscribe_in(&color_picker, window, |this, _, event, window, cx| {
                if !this.saving_tag {
                    let ColorPickerEvent::Change(color) = event;
                    this.set_tag_color(color.map(color_rgb), window, cx);
                }
            });
        let search_subscription =
            cx.subscribe(&sub_tag_search, |_, _, _: &InputEvent, cx| cx.notify());
        let (folder_tree, folder_subscription) = Self::new_folder_tree(&library, cx);
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            library,
            focus_handle,
            query: String::new(),
            active_tag: None,
            active_folder: None,
            folder_tree,
            _folder_subscription: folder_subscription,
            selected: Some(0),
            tag_input,
            editing_tag: None,
            creating_tag: false,
            editing_color: None,
            editing_sub_tags: Vec::new(),
            color_input,
            color_picker,
            sub_tag_search,
            confirming_tag_delete: false,
            saving_tag: false,
            tag_error: None,
            _tag_subscriptions: vec![
                tag_subscription,
                color_subscription,
                picker_subscription,
                search_subscription,
            ],
            _appearance_subscription: subscription,
        }
    }

    fn new_folder_tree(
        library: &DemoLibrary,
        cx: &mut Context<Self>,
    ) -> (Entity<TreeState>, Subscription) {
        let tree = cx.new(|cx| TreeState::new(cx).items(folder_items(&library.folders, None)));
        let subscription = cx.observe(&tree, |this, tree, cx| {
            let folder = tree
                .read(cx)
                .selected_item()
                .and_then(|item| item.id.parse().ok());
            if this.active_folder != folder {
                this.active_folder = folder;
                this.select_first_visible();
                cx.notify();
            }
        });
        (tree, subscription)
    }

    fn visible_files(&self) -> Vec<usize> {
        let mut files = self.library.visible_files(self.active_tag, &self.query);
        if let Some(folder) = self
            .library
            .folders
            .iter()
            .find(|folder| Some(folder.id()) == self.active_folder)
        {
            files.retain(|index| {
                self.library
                    .files
                    .get(*index)
                    .is_some_and(|file| file.info.path().starts_with(folder.path()))
            });
        }
        files
    }

    fn select_first_visible(&mut self) {
        self.selected = self.visible_files().first().copied();
    }

    fn clear_folder_filter(&mut self, cx: &mut Context<Self>) {
        self.active_folder = None;
        self.folder_tree
            .update(cx, |tree, cx| tree.set_selected_index(None, cx));
        self.select_first_visible();
        cx.notify();
    }

    fn add_tag(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_tag || name.trim().is_empty() {
            return;
        }
        let Some(index) = self.selected else { return };
        let Some(file) = self.library.files.get(index) else {
            return;
        };
        let file_name = file.info.name().to_owned();
        let save = self.library.add_tag(file.info.id(), name.clone());
        self.save_tag_change(
            save,
            format!("Could not add tag to {file_name}"),
            window,
            cx,
            move |this, (tags, tag_id), window, cx| {
                this.library.tags = tags;
                if let Some(file) = this.library.files.get_mut(index)
                    && !file.tags.contains(&tag_id)
                {
                    file.tags.push(tag_id);
                }
                if this.tag_input.read(cx).value().trim() == name.trim() {
                    this.tag_input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
            },
        );
    }

    fn remove_tag(&mut self, tag_id: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_tag {
            return;
        }
        let Some(index) = self.selected else { return };
        let Some(file) = self.library.files.get(index) else {
            return;
        };
        let save = self.library.remove_tag(file.info.id(), tag_id);
        self.save_tag_change(
            save,
            format!("Could not remove tag from {}", file.info.name()),
            window,
            cx,
            move |this, (), _, _| {
                if let Some(file) = this.library.files.get_mut(index) {
                    file.tags.retain(|id| *id != tag_id);
                }
            },
        );
    }

    fn new_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_tag {
            return;
        }
        self.cancel_tag_edit(window, cx);
        self.creating_tag = true;
        self.tag_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn edit_tag(&mut self, tag_id: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_tag {
            return;
        }
        let Some(tag) = self.library.tags.iter().find(|tag| tag.id() == tag_id) else {
            return;
        };
        let name = tag.name().to_owned();
        self.editing_tag = Some(tag_id);
        self.creating_tag = false;
        let color = tag.color();
        self.editing_sub_tags = tag.sub_tag_ids().to_vec();
        self.set_tag_color(color, window, cx);
        self.sub_tag_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirming_tag_delete = false;
        self.tag_error = None;
        self.tag_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_tag_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_tag = None;
        self.creating_tag = false;
        self.set_tag_color(None, window, cx);
        self.sub_tag_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.editing_sub_tags.clear();
        self.confirming_tag_delete = false;
        self.tag_error = None;
        self.tag_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn sync_color_picker(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.color_picker.update(cx, |picker, cx| {
            if let Some(color) = self.editing_color {
                picker.set_value(rgb(color), window, cx);
            } else {
                picker.clear_value(window, cx);
            }
        });
    }

    fn set_tag_color(&mut self, color: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_color = color;
        self.sync_color_picker(window, cx);
        self.color_input.update(cx, |input, cx| {
            input.set_value(
                color
                    .map(|color| format!("#{color:06X}"))
                    .unwrap_or_default(),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn submit_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tag_input.read(cx).value().to_string();
        if self.saving_tag || name.trim().is_empty() {
            return;
        }
        if self.creating_tag {
            let save = self.library.create_tag(name);
            self.save_tag_change(
                save,
                "Could not create tag".into(),
                window,
                cx,
                |this, tags, window, cx| {
                    this.library.tags = tags;
                    this.cancel_tag_edit(window, cx);
                },
            );
        } else if let Some(tag_id) = self.editing_tag {
            let Ok(color) = parse_tag_color(&self.color_input.read(cx).value()) else {
                return;
            };
            self.editing_color = color;
            let save = self.library.update_tag(
                tag_id,
                name,
                self.editing_color,
                self.editing_sub_tags.clone(),
            );
            self.save_tag_change(
                save,
                "Could not save tag".into(),
                window,
                cx,
                |this, tags, window, cx| {
                    this.library.tags = tags;
                    this.cancel_tag_edit(window, cx);
                },
            );
        } else {
            self.add_tag(name, window, cx);
        }
    }

    fn delete_edited_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_tag || !self.confirming_tag_delete {
            return;
        }
        let Some(tag_id) = self.editing_tag else {
            return;
        };
        let save = self.library.delete_tag(tag_id);
        self.save_tag_change(
            save,
            "Could not delete tag".into(),
            window,
            cx,
            move |this, tags, window, cx| {
                this.library.tags = tags;
                for file in &mut this.library.files {
                    file.tags.retain(|id| *id != tag_id);
                }
                if this.active_tag == Some(tag_id) {
                    this.active_tag = None;
                }
                this.cancel_tag_edit(window, cx);
            },
        );
    }

    fn save_tag_change<T: 'static>(
        &mut self,
        save: tokio::task::JoinHandle<anyhow::Result<T>>,
        error_message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_success: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.saving_tag = true;
        self.tag_error = None;
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = save.await.unwrap_or_else(|error| Err(error.into()));
            let _closed_view = view.update_in(cx, |this, window, cx| {
                this.saving_tag = false;
                match result {
                    Ok(value) => {
                        on_success(this, value, window, cx);
                        let visible = this.visible_files();
                        if !this.selected.is_some_and(|index| visible.contains(&index)) {
                            this.selected = visible.first().copied();
                        }
                    }
                    Err(error) => this.tag_error = Some(format!("{error_message}: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(file) = self
            .selected
            .and_then(|index| self.library.files.get(index))
        else {
            return;
        };
        let title = file.info.name().to_owned();
        let path = file.info.path().to_path_buf();
        let preview = cx.new(|cx| crate::preview::Preview::new(path, window, cx));
        let width = window.viewport_size().width - px(96.);
        let height = window.viewport_size().height - px(220.);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(width.min(px(1000.)))
                .margin_top(px(48.))
                .child(
                    div()
                        .id("file-preview")
                        .test_support()
                        .h(height)
                        .child(preview.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .child("Esc to close · Video: use playback controls"),
                )
        });
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_all_dialogs(cx);
        let delegate = SearchResults {
            owner: cx.weak_entity(),
            matches: self.visible_files(),
            selected: Some(IndexPath::default()),
        };
        let list = cx.new(|cx| {
            let mut list = ListState::new(delegate, window, cx).searchable(true);
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list
        });
        let focus = list.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title("Search library")
                .w(px(560.))
                .margin_top(px(100.))
                .child(
                    div()
                        .h(px(340.))
                        .child(List::new(&list).search_placeholder("Search files or tags…")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("↑ ↓ Navigate · Enter Select · Esc Close"),
                )
        });
        let query = self.query.clone();
        window.defer(cx, move |window, cx| {
            focus.update(cx, |list, cx| {
                list.set_query(&query, window, cx);
                list.focus(window, cx);
            });
        });
    }

    fn open_settings(window: &mut Window, cx: &mut Context<Self>) {
        window.close_all_dialogs(cx);
        window.open_dialog(cx, move |dialog, _, cx| {
            let appearance = cx.global::<Appearance>().0;
            dialog.title("Settings").w(px(480.)).child(
                v_flex()
                    .gap_4()
                    .py_4()
                    .child(div().font_semibold().child("Appearance"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Choose how Hestia and its Dock icon look."),
                    )
                    .child(
                        h_flex().gap_2().children(
                            [
                                ("System", None),
                                ("Light", Some(ThemeMode::Light)),
                                ("Dark", Some(ThemeMode::Dark)),
                            ]
                            .into_iter()
                            .map(|(label, mode)| {
                                Button::new(label)
                                    .label(label)
                                    .selected(appearance == mode)
                                    .toggled(appearance == mode)
                                    .on_click(move |_, window, cx| {
                                        cx.set_global(Appearance(mode));
                                        if let Some(mode) = mode {
                                            Theme::change(mode, Some(window), cx);
                                        } else {
                                            Theme::sync_system_appearance(Some(window), cx);
                                        }
                                        configure_theme(cx);
                                        window.refresh();
                                    })
                            }),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Appearance applies to this preview session."),
                    ),
            )
        });
    }

    fn search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shortcut = if cfg!(target_os = "macos") {
            "⌘ P"
        } else {
            "Ctrl P"
        };
        h_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("open-search")
                    .flex_1()
                    .min_w_0()
                    .h_9()
                    .rounded_lg()
                    .bg(cx.theme().background)
                    .border_color(cx.theme().border)
                    .accessibility_label("Search files or tags")
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(IconName::Search).size_4())
                            .child(div().flex_1().text_left().truncate().child(
                                if self.query.is_empty() {
                                    "Search files or tags…".to_owned()
                                } else {
                                    self.query.clone()
                                },
                            ))
                            .child(div().text_xs().child(shortcut)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .when(!self.query.is_empty(), |this| {
                this.child(
                    Button::new("clear-search")
                        .ghost()
                        .icon(IconName::X)
                        .accessibility_label("Clear search")
                        .tooltip("Clear search")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.query.clear();
                            this.select_first_visible();
                            cx.notify();
                        })),
                )
            })
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.library.files.len();
        v_flex()
            .id("library-sidebar")
            .h_full()
            .overflow_y_scroll()
            .p_3()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .w(px(216.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                SidebarGroup::new("LIBRARY")
                    .child(
                        SidebarMenu::new().child(
                            SidebarMenuItem::new("All files")
                                .icon(IconName::Folder)
                                .active(self.active_tag.is_none() && self.active_folder.is_none())
                                .suffix(move |_, _| div().text_xs().child(count.to_string()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.active_tag = None;
                                    this.clear_folder_filter(cx);
                                })),
                        ),
                    )
                    .render("library-group", window, cx),
            )
            .child(
                div()
                    .id("tags-section")
                    .test_support()
                    .relative()
                    .group("tags-section")
                    .child(
                        SidebarGroup::new("TAGS")
                            .child(SidebarMenu::new().children(self.library.tags.iter().map(
                                |tag| {
                                    let id = tag.id();
                                    let count = self.library.visible_files(Some(id), "").len();
                                    let view = cx.weak_entity();
                                    let saving = self.saving_tag;
                                    let label = format!("Edit {} tag", tag.name());
                                    let color = tag_color(tag, cx);
                                    SidebarMenuItem::new(tag.name().to_owned())
                                        .icon(IconName::Tag)
                                        .active(self.active_tag == Some(id))
                                        .suffix(move |_, _| {
                                            let view = view.clone();
                                            h_flex()
                                                .gap_1()
                                                .child(div().size_2().rounded_full().bg(color))
                                                .child(div().text_xs().child(count.to_string()))
                                                .child(
                                                    Button::new(("edit-tag", id as u32))
                                                        .xsmall()
                                                        .ghost()
                                                        .icon(IconName::Pencil)
                                                        .accessibility_label(label.clone())
                                                        .tooltip(label.clone())
                                                        .disabled(saving)
                                                        .on_click(move |_, window, cx| {
                                                            cx.stop_propagation();
                                                            let _closed_view =
                                                                view.update(cx, |this, cx| {
                                                                    this.edit_tag(id, window, cx);
                                                                });
                                                        }),
                                                )
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.active_tag = Some(id);
                                            this.select_first_visible();
                                            cx.notify();
                                        }))
                                },
                            )))
                            .render("tags-group", window, cx),
                    )
                    .child(
                        Button::new("create-tag")
                            .absolute()
                            .top_1()
                            .right_2()
                            .opacity(0.)
                            .group_hover("tags-section", |this| this.opacity(1.))
                            .focus_visible(|this| this.opacity(1.))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Plus)
                            .accessibility_label("Create tag")
                            .tooltip("Create tag")
                            .disabled(self.saving_tag)
                            .on_click(cx.listener(|this, _, window, cx| this.new_tag(window, cx))),
                    ),
            )
    }

    fn card(&self, index: usize, cx: &mut Context<Self>) -> Option<Button> {
        let file = self.library.files.get(index)?;
        let selected = self.selected == Some(index);
        Some(
            Button::new(("file", index))
                .accessibility_label(format!("Select {}", file.info.name()))
                .selected(selected)
                .w_full()
                .h(px(248.))
                .p_0()
                .rounded_lg()
                .bg(cx.theme().popover)
                .text_color(cx.theme().foreground)
                .border_color(if selected {
                    cx.theme().ring
                } else {
                    cx.theme().border
                })
                .child(
                    v_flex()
                        .size_full()
                        .overflow_hidden()
                        .rounded_lg()
                        .child(preview(file, 150., cx))
                        .child(
                            v_flex()
                                .gap_2()
                                .p_3()
                                .w_full()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_medium()
                                        .truncate()
                                        .child(file.info.name().to_owned()),
                                )
                                .child(
                                    h_flex()
                                        .overflow_hidden()
                                        .gap_1()
                                        .children(self.file_tags(file, false, cx)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{}  ·  {}",
                                            file.kind(),
                                            file_size(file.bytes)
                                        )),
                                ),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.selected = Some(index);
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                })),
        )
    }

    fn file_tags(&self, file: &DemoFile, removable: bool, cx: &mut Context<Self>) -> Vec<Tag> {
        self.library
            .tags
            .iter()
            .filter(|tag| file.tags.contains(&tag.id()))
            .map(|tag| {
                let color = tag_color(tag, cx);
                let foreground = if let Some(color) = tag.color() {
                    color_foreground(color)
                } else if tag.name() == "To review" {
                    cx.theme().warning_foreground
                } else {
                    cx.theme().primary_foreground
                };
                let id = tag.id();
                let remove_label = format!("Remove {} tag from this file", tag.name());
                Tag::custom(color, foreground, color)
                    .small()
                    .child(tag.name().to_owned())
                    .when(removable, |tag| {
                        tag.child(
                            Button::new(("remove-tag", id as u32))
                                .xsmall()
                                .ghost()
                                .icon(IconName::X)
                                .accessibility_label(remove_label)
                                .tooltip("Remove from this file only")
                                .disabled(self.saving_tag)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_tag(id, window, cx);
                                })),
                        )
                    })
            })
            .collect()
    }

    fn tag_editor(&self, file: Option<&DemoFile>, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self
            .library
            .tags
            .iter()
            .find(|tag| Some(tag.id()) == self.editing_tag);
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Input::new(&self.tag_input)
                            .id("tag-name")
                            .aria_label("Tag name")
                            .small()
                            .disabled(self.saving_tag),
                    )
                    .child(
                        Button::new("add-tag")
                            .small()
                            .primary()
                            .label(if self.saving_tag {
                                "Saving…"
                            } else if editing.is_some() {
                                "Save"
                            } else if self.creating_tag {
                                "Create"
                            } else {
                                "Add"
                            })
                            .accessibility_label(if editing.is_some() {
                                "Save tag changes"
                            } else if self.creating_tag {
                                "Create tag"
                            } else {
                                "Add tag to selected file"
                            })
                            .disabled(
                                self.saving_tag
                                    || (file.is_none() && editing.is_none() && !self.creating_tag)
                                    || self.tag_input.read(cx).value().trim().is_empty()
                                    || (editing.is_some()
                                        && parse_tag_color(&self.color_input.read(cx).value())
                                            .is_err()),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_tag(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex().flex_wrap().gap_1().children(
                    self.library
                        .tags
                        .iter()
                        .filter(|tag| {
                            editing.is_none()
                                && file.is_some_and(|file| !file.tags.contains(&tag.id()))
                        })
                        .map(|tag| {
                            let name = tag.name().to_owned();
                            Button::new(("assign-tag", tag.id() as u32))
                                .xsmall()
                                .icon(IconName::Plus)
                                .label(name.clone())
                                .accessibility_label(format!("Add {name} tag"))
                                .disabled(self.saving_tag)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.add_tag(name.clone(), window, cx);
                                }))
                        }),
                ),
            )
            .when(editing.is_some(), |this| {
                this.child(self.tag_options(cx))
                    .child(self.tag_delete_confirmation(cx))
            })
            .when_some(self.tag_error.clone(), |this, error| {
                this.child(
                    div()
                        .id("tag-error")
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
    }

    fn tag_delete_confirmation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_2().mt_3().pt_3().border_t_1().border_color(cx.theme().border)
            .when(self.confirming_tag_delete, |this| {
                this.child(div().text_xs().child("Delete this tag from all files and tag groups? Your files and other tags will not be deleted. This cannot be undone."))
                    .child(Button::new("keep-tag").small().label("Keep tag")
                        .disabled(self.saving_tag)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirming_tag_delete = false;
                            cx.notify();
                        })))
            })
            .child(Button::new("delete-tag").small().danger()
                .label(if self.confirming_tag_delete { "Delete permanently" } else { "Delete tag…" })
                .disabled(self.saving_tag)
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.confirming_tag_delete {
                        this.delete_edited_tag(window, cx);
                    } else {
                        this.confirming_tag_delete = true;
                        cx.notify();
                    }
                })))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative layout for tag color and sub-tag controls"
    )]
    fn tag_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let candidates = self
            .library
            .tags
            .iter()
            .filter(|tag| Some(tag.id()) != self.editing_tag);
        let searchable = candidates.clone().count() > 5;
        let query = self.sub_tag_search.read(cx).value().trim().to_lowercase();
        let matches: Vec<_> = candidates
            .filter(|tag| !searchable || tag.name().to_lowercase().contains(&query))
            .collect();
        let color_error = parse_tag_color(&self.color_input.read(cx).value()).err();
        v_flex()
            .gap_3()
            .child(section_label("COLOR", cx))
            .child(
                h_flex().flex_wrap().gap_1().children(
                    [
                        ("Auto", None),
                        ("Green", Some(0x008e_d462)),
                        ("Blue", Some(0x0070_baff)),
                        ("Coral", Some(0x00ff_998b)),
                        ("Yellow", Some(0x00f5_e211)),
                        ("Purple", Some(0x00c4_a7ef)),
                        ("Orange", Some(0x00f5_b86f)),
                    ]
                    .into_iter()
                    .map(|(label, color)| {
                        Button::new(label)
                            .xsmall()
                            .label(label)
                            .accessibility_label(format!("Tag color: {label}"))
                            .selected(self.editing_color == color)
                            .toggled(self.editing_color == color)
                            .when_some(color, |button, color| {
                                button.child(div().size_2().rounded_full().bg(rgb(color)))
                            })
                            .disabled(self.saving_tag)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_tag_color(color, window, cx);
                            }))
                    }),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when(!self.saving_tag, |this| {
                        this.child(
                            div()
                                .id("tag-color-picker")
                                .test_support()
                                .flex_shrink_0()
                                .child(
                                    ColorPicker::new(&self.color_picker)
                                        .small()
                                        .accessibility_label("Choose custom tag color"),
                                ),
                        )
                    })
                    .child(
                        Input::new(&self.color_input)
                            .id("tag-color")
                            .aria_label("Tag color: hex or OKLCH")
                            .small()
                            .flex_1()
                            .min_w_0()
                            .disabled(self.saving_tag),
                    ),
            )
            .when_some(color_error, |this, error| {
                this.child(
                    div()
                        .id("color-error")
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(section_label("SUB-TAGS", cx))
            .when(searchable, |this| {
                this.child(
                    Input::new(&self.sub_tag_search)
                        .id("sub-tag-search")
                        .aria_label("Search sub-tags")
                        .small()
                        .cleanable(true)
                        .disabled(self.saving_tag),
                )
            })
            .when(matches.is_empty(), |this| {
                this.child(
                    div()
                        .id("no-sub-tags")
                        .test_support()
                        .text_xs()
                        .child("No matching sub-tags."),
                )
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(matches.into_iter().map(|tag| {
                        let id = tag.id();
                        let selected = self.editing_sub_tags.contains(&id);
                        Button::new(("sub-tag", id as u32))
                            .xsmall()
                            .label(tag.name().to_owned())
                            .icon(if selected {
                                IconName::Check
                            } else {
                                IconName::Plus
                            })
                            .selected(selected)
                            .toggled(selected)
                            .accessibility_label(format!("Include {} as a sub-tag", tag.name()))
                            .disabled(self.saving_tag)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.editing_sub_tags.contains(&id) {
                                    this.editing_sub_tags.retain(|tag| *tag != id);
                                } else {
                                    this.editing_sub_tags.push(id);
                                }
                                this.tag_error = None;
                                cx.notify();
                            }))
                    })),
            )
    }

    fn folders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .flex_shrink_0()
            .pb_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .justify_between()
                    .child(section_label("FOLDERS", cx))
                    .child(
                        Button::new("all-folders")
                            .xsmall()
                            .ghost()
                            .label("All folders")
                            .disabled(self.active_folder.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.clear_folder_filter(cx))),
                    ),
            )
            .child(
                div()
                    .id("folder-tree")
                    .test_support()
                    .h(px(180.))
                    .child(Tree::new(&self.folder_tree, |_, entry, _, _, _| {
                        ListItem::new(format!("folder-{}", entry.item().id))
                            .pl(px(entry.depth() as f32 * 16.))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .min_w_0()
                                    .child(
                                        Icon::new(if entry.is_expanded() {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size_3()
                                        .when(!entry.is_folder(), gpui_kit::Styled::invisible),
                                    )
                                    .child(
                                        Icon::new(if entry.is_expanded() {
                                            IconName::FolderOpen
                                        } else {
                                            IconName::Folder
                                        })
                                        .size_4(),
                                    )
                                    .child(div().truncate().child(entry.item().label.clone())),
                            )
                    })),
            )
    }

    fn details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = v_flex()
            .id("file-details")
            .overflow_y_scroll()
            .w(px(264.))
            .h_full()
            .flex_shrink_0()
            .p_5()
            .gap_5()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .child(self.folders(cx))
            .child(h_flex().justify_between().text_sm().font_medium().child(
                if self.creating_tag {
                    "New tag"
                } else if self.editing_tag.is_some() {
                    "Edit tag"
                } else {
                    "File details"
                },
            ));
        if self.editing_tag.is_some() || self.creating_tag {
            return panel.child(self.tag_editor(None, cx));
        }
        let Some(file) = self
            .selected
            .and_then(|index| self.library.files.get(index))
        else {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Select a file to take a closer look."),
                )
                .when(self.tag_error.is_some(), |panel| {
                    panel.child(self.tag_editor(None, cx))
                });
        };
        panel
            .child(
                div()
                    .rounded_lg()
                    .overflow_hidden()
                    .child(preview(file, 164., cx)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(file.info.name().to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(file.kind()),
                    ),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(section_label("TAGS", cx))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .children(self.file_tags(file, true, cx)),
                    )
                    .child(self.tag_editor(Some(file), cx)),
            )
            .child(
                v_flex()
                    .gap_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(section_label("INFORMATION", cx))
                    .child(detail_row("Size", &file_size(file.bytes), cx))
                    .child(detail_row("Library", "Learning library", cx))
                    .child(detail_row("Source", "Demo files", cx)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(section_label("LOCATION", cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.library.file_location(file)),
                    ),
            )
    }
}

impl Render for FileManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialogs = Root::render_dialog_layer(window, cx);
        let visible = self.visible_files();
        let title = self
            .library
            .tags
            .iter()
            .find(|tag| Some(tag.id()) == self.active_tag)
            .map(controllers::TagInfo::name)
            .or_else(|| {
                self.library.folders.iter()
                    .find(|folder| Some(folder.id()) == self.active_folder)
                    .map(controllers::FolderInfo::name)
            })
            .unwrap_or("All files")
            .to_owned();
        let columns = if window.viewport_size().width < px(1240.) {
            2
        } else {
            3
        };
        v_flex().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
            .key_context("FileManager")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &crate::platform::OpenPreview, window, cx| {
                if this.focus_handle.is_focused(window) {
                    this.open_preview(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(h_flex().flex_1().min_h_0()
                .child(self.sidebar(window, cx))
                .child(v_flex().id("file-content").test_support().flex_1().min_w_0().h_full()
                    .child(v_flex().p_6().gap_5().flex_shrink_0()
                        .child(self.search_bar(cx))
                        .child(h_flex().justify_between().items_start()
                            .child(div().text_3xl().font_semibold().child(title))
                            .child(Tag::primary().child("Demo library")))
                        .child(h_flex().justify_end()
                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("Files: {} · A–Z", visible.len())))))
                    .child(div().id("file-grid-scroll").flex_1().min_h_0().overflow_y_scroll().px_6().pb_6()
                        .child(div().grid().grid_cols(columns).gap_4().children(visible.iter().filter_map(|index| self.card(*index, cx))))
                        .when(visible.is_empty(), |this| this.child(v_flex().p_10().items_center().gap_3()
                            .child(Icon::new(IconName::Search).size_8().text_color(cx.theme().muted_foreground))
                            .child(div().text_lg().child("No matching files"))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Try another name or choose a different tag.")))))
                )
                .child(self.details(cx)))
            .child(h_flex().h_8().flex_shrink_0().px_4().gap_2().border_t_1().border_color(cx.theme().border).bg(cx.theme().popover).text_xs().text_color(cx.theme().muted_foreground)
                .child(div().size_1p5().rounded_full().bg(cx.theme().primary))
                .child("Local & offline")
                .child(div().flex_1())
                .child(format!("{} sample files · Space to preview · No changes to your files", self.library.files.len())))
            .children(dialogs)
    }
}

// ponytail: scan the demo folders per level; index by parent for large libraries.
fn folder_items(folders: &[controllers::FolderInfo], parent: Option<i32>) -> Vec<TreeItem> {
    folders
        .iter()
        .filter(|folder| folder.parent_id() == parent)
        .map(|folder| {
            TreeItem::new(folder.id().to_string(), folder.name().to_owned())
                .expanded(parent.is_none())
                .children(folder_items(folders, Some(folder.id())))
        })
        .collect()
}

struct SearchResults {
    owner: WeakEntity<FileManager>,
    matches: Vec<usize>,
    selected: Option<IndexPath>,
}

impl ListDelegate for SearchResults {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len()
    }

    fn perform_search(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.matches = self
            .owner
            .update(cx, |this, cx| {
                query.clone_into(&mut this.query);
                this.select_first_visible();
                cx.notify();
                this.visible_files()
            })
            .unwrap_or_default();
        // List chooses from its previous row cache during search. Reset after that
        // so a query that follows an empty result can still be confirmed with Enter.
        cx.defer_in(window, |list, window, cx| {
            let first = (!list.delegate().matches.is_empty()).then_some(IndexPath::default());
            list.set_selected_index(first, window, cx);
            cx.notify();
        });
        cx.notify();
        Task::ready(())
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let index = *self.matches.get(ix.row)?;
        let owner = self.owner.upgrade()?;
        let file = owner.read(cx).library.files.get(index)?;
        Some(
            ListItem::new(("search-result", index)).child(
                h_flex()
                    .gap_3()
                    .py_2()
                    .child(Icon::new(IconName::FileText).size_4())
                    .child(
                        v_flex().gap_1().child(file.info.name().to_owned()).child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(file.kind()),
                        ),
                    ),
            ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_6()
            .text_color(cx.theme().muted_foreground)
            .child("No matching files. Try a name or tag.")
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        if let Some(index) = self
            .selected
            .and_then(|ix| self.matches.get(ix.row))
            .copied()
        {
            let _dropped_view = self.owner.update(cx, |this, cx| {
                this.selected = Some(index);
                cx.notify();
            });
            window.close_dialog(cx);
        }
    }
}

fn color_foreground(color: u32) -> Hsla {
    let color = rgb(color);
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b);
    rgb(if luminance > 0.179 { 0 } else { 0x00ff_ffff }).into()
}

fn color_rgb(color: Hsla) -> u32 {
    let color = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

fn parse_tag_color(value: &str) -> Result<Option<u32>, &'static str> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(None);
    }
    let error = "Enter #RGB, #RRGGBB or oklch(L C H), without transparency.";
    if let Some(body) = value
        .strip_prefix("oklch(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = body.split_whitespace();
        let number = |part: &str, percent_scale: f32| -> Option<f32> {
            let value = if let Some(percent) = part.strip_suffix('%') {
                percent.parse::<f32>().ok()? * percent_scale / 100.
            } else {
                part.parse::<f32>().ok()?
            };
            value.is_finite().then_some(value)
        };
        let lightness = number(parts.next().ok_or(error)?, 1.).ok_or(error)?;
        let chroma = number(parts.next().ok_or(error)?, 0.4).ok_or(error)?;
        let hue = parts.next().ok_or(error)?;
        let hue = hue
            .strip_suffix("deg")
            .unwrap_or(hue)
            .parse::<f32>()
            .map_err(|_| error)?;
        if parts.next().is_some()
            || !(0. ..=1.).contains(&lightness)
            || chroma < 0.
            || !hue.is_finite()
        {
            return Err(error);
        }
        let color = oklch(lightness, chroma, hue.rem_euclid(360.));
        if ![color.h, color.s, color.l].into_iter().all(f32::is_finite) {
            return Err(error);
        }
        return Ok(Some(color_rgb(color)));
    }
    let hex = value.strip_prefix('#').unwrap_or(&value);
    if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error);
    }
    let hex = if hex.len() == 3 {
        hex.chars()
            .flat_map(|digit| [digit, digit])
            .collect::<String>()
    } else {
        hex.to_owned()
    };
    u32::from_str_radix(&hex, 16).map(Some).map_err(|_| error)
}

fn tag_color(tag: &controllers::TagInfo, cx: &App) -> Hsla {
    if let Some(color) = tag.color() {
        return rgb(color).into();
    }
    match tag.name() {
        "Learning" => cx.theme().primary,
        "Notes" => cx.theme().info,
        "Reference" => cx.theme().danger,
        _ => cx.theme().warning,
    }
}

fn section_label(label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .font_medium()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

fn detail_row(label: &'static str, value: &str, cx: &App) -> impl IntoElement {
    h_flex()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(value.to_owned())
}

fn file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.)
    }
}

fn preview(file: &DemoFile, height: f32, cx: &App) -> impl IntoElement {
    let image = file.kind() == "PNG image";
    div()
        .w_full()
        .h(px(height))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(cx.theme().background)
        .p_4()
        .flex()
        .items_center()
        .justify_center()
        .when(image, |this| {
            this.child(
                img(file.info.path().to_path_buf())
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Contain),
            )
        })
        .when(!image, |this| {
            this.child(
                v_flex()
                    .w_full()
                    .h_full()
                    .gap_3()
                    .p_3()
                    .bg(cx.theme().popover)
                    .rounded_sm()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(IconName::FileText).small())
                            .child(div().text_xs().font_medium().child(file.kind())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(cx.theme().foreground)
                            .child(file.excerpt),
                    ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::expect_used,
        clippy::too_many_lines,
        reason = "Exercise folder hierarchy, keyboard navigation and combined filters in one UI scenario"
    )]
    #[gpui_kit::test]
    fn browse_folders_and_subfolders(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;

        let runtime = tokio::runtime::Runtime::new().expect("Runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Library");
        let folder_id = |name| {
            library
                .folders
                .iter()
                .find(|folder| folder.name() == name)
                .expect("Folder")
                .id()
        };
        let documents = folder_id("Documents");
        let notes = folder_id("Notes");
        let archive = folder_id("Archive");
        let learning = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Learning")
            .expect("Learning tag")
            .id();
        cx.update(|cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(gpui_kit::size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("Manager");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("folder-tree").visible());
            assert!(window.find(format!("folder-{archive}")).visible());
            assert!(window.try_find(format!("folder-{notes}")).is_none());
            window.click(format!("folder-{documents}"), cx);
        })
        .expect("Expand Documents");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).active_folder, Some(documents));
            assert_eq!(
                view.read(cx).visible_files().len(),
                4,
                "Includes descendants"
            );
            assert!(window.find(format!("folder-{notes}")).visible());
            window.press("left", cx);
            assert!(window.try_find(format!("folder-{notes}")).is_none());
            window.press("right", cx);
            assert!(window.find(format!("folder-{notes}")).visible());
            window.press("down", cx);
        })
        .expect("Keyboard expansion and subfolder selection");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert_eq!(manager.active_folder, Some(notes));
            assert_eq!(manager.visible_files().len(), 2);
            assert_eq!(
                manager
                    .folder_tree
                    .read(cx)
                    .selected_entry()
                    .expect("Entry")
                    .depth(),
                2
            );
            window.click("open-search", cx);
        })
        .expect("Search inside Notes");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.input("Project", cx);
        })
        .expect("Search for a file outside the selected folder");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(view.read(cx).visible_files().is_empty());
            assert!(view.read(cx).selected.is_none());
            window.press("escape", cx);
            window.click("clear-search", cx);
            assert_eq!(view.read(cx).visible_files().len(), 2);
            view.update(cx, |manager, cx| {
                manager.active_tag = Some(learning);
                manager.select_first_visible();
                assert_eq!(
                    manager.visible_files().len(),
                    1,
                    "Tag and folder filters intersect"
                );
                manager.active_tag = None;
                cx.notify();
            });
            window.click(format!("folder-{archive}"), cx);
        })
        .expect("Combine filters and select an empty folder");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).active_folder, Some(archive));
            assert!(view.read(cx).visible_files().is_empty());
            assert!(view.read(cx).selected.is_none());
            assert!(window.find("folder-tree").visible());
            window.click("all-folders", cx);
            assert!(view.read(cx).active_folder.is_none());
            assert_eq!(view.read(cx).visible_files().len(), 7);
        })
        .expect("Clear the folder filter");
    }

    #[expect(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::too_many_lines,
        reason = "One end-to-end UI scenario; failed setup and missing selections must fail the test"
    )]
    #[gpui_kit::test]
    fn search_in_file_pane_and_settings_shortcut_work(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::size;
        use gpui_kit::test::TestWindowExt as _;

        let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Demo library");
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::platform::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("File manager");
        let primary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let search = window.find("open-search").bounds();
            let content = window.find("file-content").bounds();
            assert_eq!(content.origin.y, px(0.));
            assert_eq!(search.center().x, content.center().x);
            assert!(search.origin.x > content.origin.x);
            assert!(search.right() < content.right());
            assert!(window.try_find("open-settings").is_none());
            assert!(window.try_find("theme-toggle").is_none());
            window.press(&format!("{primary}-p"), cx);
        })
        .expect("Open search");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("dialog").visible());
            assert!(window.has_focused_input(cx));
            window.input("PROJECT", cx);
        })
        .expect("Type search");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let manager = view.read(cx);
            assert_eq!(manager.query, "PROJECT");
            assert_eq!(manager.library.visible_files(None, &manager.query).len(), 1);
            window.press("enter", cx);
        })
        .expect("Confirm result");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            let manager = view.read(cx);
            assert_eq!(
                manager.library.files[manager.selected.expect("Selection")]
                    .info
                    .name(),
                "Project checklist.md"
            );
            window.press(&format!("{primary}-,"), cx);
        })
        .expect("Open settings");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("dialog").visible());
            for (label, mode) in [("Light", ThemeMode::Light), ("Dark", ThemeMode::Dark)] {
                window.click(label, cx);
                assert_eq!(cx.theme().mode, mode);
                assert_eq!(cx.global::<Appearance>().0, Some(mode));
                assert_eq!(window.find(label).checked(), Some(true));
            }
            window.click("System", cx);
            assert!(cx.global::<Appearance>().0.is_none());
            window.press("escape", cx);
        })
        .expect("Change appearance");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            window.click("clear-search", cx);
            window.click("open-search", cx);
        })
        .expect("Reopen from file pane");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .expect("Keyboard navigation");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).selected, Some(1));
            window.press(&format!("{primary}-p"), cx);
        })
        .expect("Reopen search");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.input("no-such-file", cx);
        })
        .expect("Empty results");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).selected.is_none());
            window.press("enter", cx);
            assert!(window.has_active_dialog(cx));
            window.press(&format!("{primary}-a"), cx);
            window.input("ideas", cx);
        })
        .expect("Replace empty query");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .expect("Confirm after empty results");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            window.click("open-search", cx);
        })
        .expect("Reopen with existing query");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).query, "ideas");
            window.press("escape", cx);
        })
        .expect("Dismiss search");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .expect("Search closed");
    }

    #[expect(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "UI scenario must fail on missing controls, tags, or selections"
    )]
    #[gpui_kit::test]
    async fn add_tags_from_details(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::size;
        use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
        use std::time::Duration;

        let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Demo library");
        let notes_id = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Notes")
            .expect("Notes")
            .id();
        cx.update(|cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("File manager");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("add-tag", cx);
            assert!(!view.read(cx).saving_tag);
            window.click(("assign-tag", notes_id as u32), cx);
        })
        .expect("Assign existing tag");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(manager.library.files[0].tags.contains(&notes_id));
            window.click("tag-name", cx);
            window.input("  Personal  ", cx);
            window.press("enter", cx);
        })
        .expect("Create tag with Enter");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert_eq!(manager.library.tags.len(), 5);
            assert_eq!(manager.library.visible_files(None, "Personal"), vec![0]);
            assert!(manager.tag_input.read(cx).value().is_empty());
            window.click("tag-name", cx);
            window.input("Personal", cx);
            window.click("add-tag", cx);
            // Completion must still update the original file, not the new selection.
            view.update(cx, |manager, cx| {
                manager.selected = Some(1);
                cx.notify();
            });
        })
        .expect("Duplicate tag and selection change");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, _, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert_eq!(manager.library.visible_files(None, "Personal"), vec![0]);
            assert_eq!(manager.library.files[0].tags.len(), 4);
            assert_eq!(manager.library.tags.len(), 5);
        })
        .expect("No duplicate or wrong-file assignment");
    }

    #[expect(
        clippy::expect_used,
        reason = "UI scenario requires controls and persisted tags"
    )]
    #[gpui_kit::test]
    async fn create_tag_from_sidebar(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::InputEvent as _;
        use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
        use std::time::Duration;

        let runtime = tokio::runtime::Runtime::new().expect("Runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Library");
        cx.update(|cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(gpui_kit::size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("Manager");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.find("create-tag").visible());
            window.hover("tags-section", cx);
            assert!(window.find("create-tag").visible());
            window.hover("open-search", cx);
            assert!(!window.find("create-tag").visible());
            // Tab navigation must reveal and activate the hover-only action too.
            for _ in 0..30 {
                window.press("tab", cx);
                if window.find("create-tag").focused() == Some(true) {
                    break;
                }
            }
            assert_eq!(window.find("create-tag").focused(), Some(true));
            assert!(window.find("create-tag").visible());
            window.press("enter", cx);
            window.dispatch_event(
                gpui_kit::KeyUpEvent {
                    keystroke: gpui_kit::Keystroke::parse("enter").expect("Enter key"),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(view.read(cx).creating_tag);
            assert!(window.has_focused_input(cx));
            window.press("enter", cx);
            assert!(!view.read(cx).saving_tag);
            window.input("  Personal  ", cx);
            window.press("enter", cx);
        })
        .expect("Hover and keyboard creation");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(!manager.creating_tag);
            let tag = manager
                .library
                .tags
                .iter()
                .find(|tag| tag.name() == "Personal")
                .expect("New tag");
            assert!(manager.library.visible_files(Some(tag.id()), "").is_empty());
            assert!(window.find(("edit-tag", tag.id() as u32)).visible());
            view.update(cx, |manager, cx| {
                manager.selected = None;
                cx.notify();
            });
            window.hover("tags-section", cx);
            window.click("create-tag", cx);
            window.input("Personal", cx);
            window.click("add-tag", cx);
        })
        .expect("Create without a selected file");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, _, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(!manager.creating_tag);
            assert_eq!(manager.library.tags.len(), 5, "Existing tags are reused");
        })
        .expect("Duplicate name does not create a second tag");
    }

    #[expect(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::too_many_lines,
        reason = "End-to-end tag editing scenario must fail on missing controls or data"
    )]
    #[gpui_kit::test]
    async fn remove_and_rename_tags(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::size;
        use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
        use std::time::Duration;

        let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Demo library");
        let id = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Reference")
            .expect("Reference")
            .id();
        let notes_id = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Notes")
            .expect("Notes")
            .id();
        let file_count = library.files.len();
        cx.update(|cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| {
                let mut manager = FileManager::new(library, window, cx);
                manager.active_tag = Some(id);
                manager.query = "Reference".into();
                manager
            });
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("File manager");
        let select_all = if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("remove-tag", id as u32), cx);
        })
        .expect("Remove from selected file");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(!manager.library.files[0].tags.contains(&id));
            assert!(manager.library.files[1].tags.contains(&id));
            assert_eq!(manager.library.visible_files(Some(id), "").len(), 2);
            assert_eq!(manager.selected, Some(1));
            assert_eq!(manager.library.tags.len(), 4);
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(view.read(cx).tag_input.read(cx).value(), "Reference");
            assert!(window.has_focused_input(cx));
            window.press(select_all, cx);
            window.input("  ", cx);
            window.press("enter", cx);
            assert!(!view.read(cx).saving_tag);
            window.click("Blue", cx);
            window.click(("sub-tag", notes_id as u32), cx);
            assert!(window.try_find("cancel-tag-edit").is_none());
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(view.read(cx).editing_color, None);
            assert!(view.read(cx).editing_sub_tags.is_empty());
            assert!(window.try_find(("sub-tag", id as u32)).is_none());
            window.press(select_all, cx);
            window.input("Notes", cx);
            window.click("add-tag", cx);
        })
        .expect("Reset empty rename and reject duplicate");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(
                manager
                    .tag_error
                    .as_ref()
                    .expect("Rename error")
                    .contains("already exists")
            );
            assert_eq!(manager.editing_tag, Some(id));
            assert_eq!(manager.tag_input.read(cx).value(), "Notes");
            assert!(window.find("tag-error").visible());
            window.click("tag-name", cx);
            window.press(select_all, cx);
            window.input("  Research  ", cx);
            window.click("Purple", cx);
            window.click(("sub-tag", notes_id as u32), cx);
            assert_eq!(window.find("Purple").checked(), Some(true));
            assert_eq!(
                window.find(("sub-tag", notes_id as u32)).checked(),
                Some(true)
            );
            window.click("tag-name", cx);
            window.press("enter", cx);
        })
        .expect("Rename with Enter");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(manager.editing_tag.is_none());
            assert!(manager.selected.is_none());
            assert_eq!(manager.active_tag, Some(id));
            assert!(manager.library.visible_files(None, "Reference").is_empty());
            assert_eq!(manager.library.visible_files(None, "Research").len(), 2);
            assert!(
                manager
                    .library
                    .tags
                    .iter()
                    .any(|tag| tag.id() == id && tag.name() == "Research")
            );
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(view.read(cx).tag_input.read(cx).value(), "Research");
            assert_eq!(view.read(cx).editing_color, Some(0x00c4_a7ef));
            assert_eq!(view.read(cx).editing_sub_tags, vec![notes_id]);
            let tag = view
                .read(cx)
                .library
                .tags
                .iter()
                .find(|tag| tag.id() == id)
                .expect("Research tag");
            assert_eq!(tag_color(tag, cx), rgb(0x00c4_a7ef).into());
            window.click("add-tag", cx);
        })
        .expect("Can edit even with no selected file");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, _, cx| {
            assert!(view.read(cx).tag_error.is_none());
            assert!(view.read(cx).editing_tag.is_none());
        })
        .expect("Unchanged name is valid");
        cx.update_window(handle.into(), |_, window, cx| {
            window.click(("edit-tag", id as u32), cx);
            window.click(("sub-tag", notes_id as u32), cx);
            window.click("Auto", cx);
            window.click("add-tag", cx);
        })
        .expect("Clear color and sub-tags");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(view.read(cx).editing_color, None);
            assert!(view.read(cx).editing_sub_tags.is_empty());
            window.click("delete-tag", cx);
            assert!(view.read(cx).confirming_tag_delete);
            assert!(!view.read(cx).saving_tag);
            window.click("keep-tag", cx);
            assert!(!view.read(cx).confirming_tag_delete);
            assert_eq!(view.read(cx).library.tags.len(), 4);
            window.click("delete-tag", cx);
            window.click("delete-tag", cx);
        })
        .expect("Delete only after confirmation");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, _, cx| {
            let manager = view.read(cx);
            assert!(manager.tag_error.is_none());
            assert!(manager.editing_tag.is_none());
            assert!(manager.active_tag.is_none());
            assert_eq!(manager.library.tags.len(), 3);
            assert_eq!(manager.library.files.len(), file_count);
            assert!(
                manager
                    .library
                    .files
                    .iter()
                    .all(|file| !file.tags.contains(&id))
            );
            assert!(
                manager
                    .library
                    .tags
                    .iter()
                    .all(|tag| tag.id() != id && !tag.sub_tag_ids().contains(&id))
            );
        })
        .expect("Deletion removes references but preserves files");
    }

    #[expect(
        clippy::expect_used,
        reason = "UI setup and interactions must fail the test on error"
    )]
    #[gpui_kit::test]
    async fn space_previews_files_without_interrupting_typing(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
        use std::time::Duration;

        let runtime = tokio::runtime::Runtime::new().expect("Runtime");
        let library = runtime.block_on(DemoLibrary::load()).expect("Library");
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::platform::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(gpui_kit::size(px(1320.), px(860.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("Manager");
        for (name, id) in [
            ("Learning notes.md", "preview-markdown"),
            ("Reading list.txt", "preview-text"),
            ("Class composition.png", "preview-image"),
        ] {
            cx.update_window(handle.into(), |_, window, cx| {
                let index = view.update(cx, |manager, cx| {
                    manager.query = name.into();
                    manager.select_first_visible();
                    cx.notify();
                    manager.selected.expect("Selected file")
                });
                window.render_frame(cx);
                window.click(("file", index), cx);
                window.press("space", cx);
            })
            .expect("Open preview with Space");
            cx.wait_for(handle.into(), Duration::from_secs(5), |window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .await;
            cx.update_window(handle.into(), |_, window, cx| {
                assert!(window.find("file-preview").visible());
                window.press("escape", cx);
            })
            .expect("Close preview");
            cx.run_until_parked();
        }
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            window.click("tag-name", cx);
            window.input("two", cx);
            window.press("space", cx);
            window.input("words", cx);
            assert!(!window.has_active_dialog(cx));
            assert_eq!(view.read(cx).tag_input.read(cx).value(), "two words");
            view.update(cx, |manager, cx| {
                manager.selected = None;
                manager.focus_handle.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            window.press("space", cx);
            assert!(!window.has_active_dialog(cx));
        })
        .expect("Typing and empty selection do not open a preview");
    }

    #[test]
    fn custom_color_formats_and_contrast() {
        for (text, expected) in [
            ("", None),
            ("  ", None),
            (" #aBc ", Some(0x00aa_bbcc)),
            ("123456", Some(0x0012_3456)),
            ("#000000", Some(0)),
            ("oklch(0 0 0)", Some(0)),
            ("OKLCH(100% 0 0)", Some(0x00ff_ffff)),
            ("oklch(50% 0 250deg)", Some(0x0063_6363)),
            ("oklch(0.627955 0.257683 29.2339)", Some(0x00ff_0000)),
        ] {
            assert_eq!(parse_tag_color(text), Ok(expected), "{text}");
        }
        assert_eq!(
            parse_tag_color("oklch(70% 50% 250)"),
            parse_tag_color("oklch(0.7 0.2 610)")
        );
        for text in [
            "#12",
            "#gggggg",
            "#12345678",
            "#abcé",
            "oklch()",
            "oklch(1 0)",
            "oklch(1 0 0) junk",
            "oklch(NaN 0 0)",
            "oklch(0.5 inf 0)",
            "oklch(0.5 0 NaN)",
            "oklch(101% 0 0)",
            "oklch(0.5 -0.1 0)",
            "oklch(0.5 0 0 / 0.5)",
            "oklch(0.5 0 20%)",
        ] {
            assert!(parse_tag_color(text).is_err(), "{text}");
        }
        assert_eq!(color_foreground(0), rgb(0x00ff_ffff).into());
        assert_eq!(color_foreground(0x00ff_ffff), rgb(0).into());
    }

    #[expect(
        clippy::expect_used,
        clippy::too_many_lines,
        reason = "UI scenario checks validation, persistence, picker integration and the search threshold"
    )]
    #[gpui_kit::test]
    async fn custom_colors_and_sub_tag_search(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
        use std::time::Duration;

        let runtime = tokio::runtime::Runtime::new().expect("Runtime");
        let mut library = runtime.block_on(DemoLibrary::load()).expect("Library");
        let file_id = library.files.first().expect("File").info.id();
        for name in ["Personal", "Work"] {
            library.tags = runtime
                .block_on(library.add_tag(file_id, name.into()))
                .expect("Task")
                .expect("Tag")
                .0;
        }
        let id = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Reference")
            .expect("Reference")
            .id();
        let notes_id = library
            .tags
            .iter()
            .find(|tag| tag.name() == "Notes")
            .expect("Notes")
            .id();
        cx.update(|cx| {
            gpui_kit::init(cx);
            configure_theme(cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(gpui_kit::size(px(1320.), px(1000.)), |window, cx| {
            let manager = cx.new(|cx| FileManager::new(library, window, cx));
            view = Some(manager.clone());
            Root::new(manager, window, cx)
        });
        let view = view.expect("Manager");
        let select_all = if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("edit-tag", id as u32), cx);
            assert!(
                window.try_find("sub-tag-search").is_none(),
                "Five candidates need no search"
            );
            window.click("tag-color", cx);
            window.input("#123456", cx);
        })
        .expect("Enter hex");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(view.read(cx).editing_color, Some(0x0012_3456));
            assert_eq!(
                view.read(cx).color_picker.read(cx).value().map(color_rgb),
                Some(0x0012_3456)
            );
            window.press(select_all, cx);
            window.input("invalid", cx);
            window.click("add-tag", cx);
            view.update(cx, |manager, cx| manager.submit_tag(window, cx));
            assert!(!view.read(cx).saving_tag);
            assert!(window.find("color-error").visible());
            window.click("tag-color", cx);
            window.press(select_all, cx);
            window.input("oklch(50% 0 0)", cx);
            window.click("add-tag", cx);
        })
        .expect("Reject invalid input and save OKLCH");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(view.read(cx).tag_error.is_none());
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(view.read(cx).editing_color, Some(0x0063_6363));
            assert_eq!(view.read(cx).color_input.read(cx).value(), "#636363");
            let swatch = window.find("tag-color-picker").bounds();
            let input = window.find("tag-color").bounds();
            assert!((swatch.center().y - input.center().y).abs() < px(1.));
            assert!(swatch.right() < input.origin.x);
            let picker = view.read(cx).color_picker.clone();
            window.click("tag-color-picker", cx);
            assert!(picker.read(cx).is_open());
            let color = cx.theme().red;
            picker.update(cx, |picker, cx| picker.select_color(color, window, cx));
        })
        .expect("Select from picker");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(view.read(cx).editing_color, Some(color_rgb(cx.theme().red)));
            assert_eq!(
                parse_tag_color(&view.read(cx).color_input.read(cx).value()),
                Ok(view.read(cx).editing_color)
            );
            window.click("Auto", cx);
            assert!(view.read(cx).color_input.read(cx).value().is_empty());
            view.update(cx, |manager, cx| {
                manager.add_tag("Sixth candidate".into(), window, cx);
            });
        })
        .expect("Reset color and add sixth candidate");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            window.click(("edit-tag", id as u32), cx);
            assert_eq!(
                view.read(cx).editing_color,
                Some(0x0063_6363),
                "Reopening the editor discards unsaved picker changes"
            );
            assert!(window.find("sub-tag-search").visible());
            window.click("sub-tag-search", cx);
            window.input("  nOtEs  ", cx);
        })
        .expect("Search six candidates");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("sub-tag", id as u32)).is_none());
            assert!(window.try_find(("sub-tag", notes_id as u32)).is_some());
            assert!(
                window.try_find("sub-tag-search").is_some(),
                "Search stays visible after filtering"
            );
            window.click(("sub-tag", notes_id as u32), cx);
            window.click("sub-tag-search", cx);
            window.press(select_all, cx);
            window.input("no-such-tag", cx);
        })
        .expect("Select filtered sub-tag");
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("sub-tag", notes_id as u32)).is_none());
            assert!(window.find("no-sub-tags").visible());
            assert_eq!(view.read(cx).editing_sub_tags, vec![notes_id]);
            window.click("add-tag", cx);
        })
        .expect("Hidden selections are retained");
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
            !view.read(cx).saving_tag
        })
        .await;
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(view.read(cx).tag_error.is_none());
            window.click(("edit-tag", id as u32), cx);
            assert!(view.read(cx).sub_tag_search.read(cx).value().is_empty());
            assert_eq!(view.read(cx).editing_sub_tags, vec![notes_id]);
        })
        .expect("Selection persisted and search reset");
    }

    #[test]
    fn palettes_match_expected_colors_and_refresh_component_tokens() {
        let mut theme = Theme::default();
        for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Light] {
            theme.mode = mode;
            apply_palette(&mut theme);
            let expected = if mode.is_dark() {
                [
                    0x0017_1a15,
                    0x00f5_f0e7,
                    0x0026_2c23,
                    0x0049_5242,
                    0x00b8_b2a7,
                    0x0038_3a20,
                ]
            } else {
                [
                    0x00f3_f4f5,
                    0x002c_2e30,
                    0x00ff_ffff,
                    0x00d5_d7da,
                    0x0062_666b,
                    0x00f5_e211,
                ]
            };
            assert_eq!(
                [
                    theme.background,
                    theme.foreground,
                    theme.popover,
                    theme.border,
                    theme.muted_foreground,
                    theme.warning
                ],
                expected.map(|color| rgb(color).into()),
            );
            assert_eq!(theme.primary, rgb(0x008e_d462).into());
            assert_eq!(theme.primary_foreground, rgb(0x002c_2e2a).into());
            assert_eq!(theme.info, rgb(0x002b_a0ff).into());
            assert_eq!(theme.danger, rgb(0x00ff_705d).into());
            assert_eq!(theme.sidebar_foreground, theme.foreground);
            assert_eq!(theme.button_foreground, theme.foreground);
            assert_eq!(theme.tokens.sidebar.color, theme.popover);
            assert_eq!(theme.tokens.sidebar_accent.color, theme.primary);
            assert_eq!(theme.tokens.button.color, theme.popover);
            assert_eq!(theme.tokens.button_primary.color, theme.primary);
            assert_eq!(theme.tokens.scrollbar_thumb.color, theme.border);
        }
    }
}
