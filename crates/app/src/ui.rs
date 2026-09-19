use crate::demo::{DemoFile, DemoLibrary};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, Theme, ThemeColor,
    ThemeMode,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Window, div, img, prelude::FluentBuilder as _, px, rgb,
};

pub(crate) fn configure_theme(cx: &mut App) {
    apply_palette(Theme::global_mut(cx));
    Theme::sync_base(cx);
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
    search: Entity<InputState>,
    active_tag: Option<i32>,
    selected: Option<usize>,
    _search_subscription: Subscription,
}

impl FileManager {
    pub(crate) fn new(library: DemoLibrary, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search files or tags…"));
        let subscription = cx.subscribe_in(&search, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.select_first_visible(cx);
                cx.notify();
            }
        });
        Self {
            library,
            search,
            active_tag: None,
            selected: Some(0),
            _search_subscription: subscription,
        }
    }

    fn select_first_visible(&mut self, cx: &App) {
        self.selected = self
            .library
            .visible_files(self.active_tag, &self.search.read(cx).value())
            .first()
            .copied();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.library.files.len();
        Sidebar::new("library-sidebar")
            .w(px(216.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .header(
                SidebarHeader::new()
                    .p_5()
                    .gap_3()
                    .child(
                        div()
                            .size_9()
                            .rounded_lg()
                            .bg(cx.theme().primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().primary_foreground)
                            .child(Icon::new(IconName::Layers).size_5()),
                    )
                    .child(div().text_lg().font_semibold().child("hestia")),
            )
            .child(
                SidebarGroup::new("LIBRARY").child(
                    SidebarMenu::new().child(
                        SidebarMenuItem::new("All files")
                            .icon(IconName::Folder)
                            .active(self.active_tag.is_none())
                            .suffix(move |_, _| div().text_xs().child(count.to_string()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tag = None;
                                this.select_first_visible(cx);
                                cx.notify();
                            })),
                    ),
                ),
            )
            .child(SidebarGroup::new("TAGS").child(SidebarMenu::new().children(
                self.library.tags.iter().map(|tag| {
                    let id = tag.id();
                    let count = self.library.visible_files(Some(id), "").len();
                    SidebarMenuItem::new(tag.name().to_owned())
                        .icon(IconName::Tag)
                        .active(self.active_tag == Some(id))
                        .suffix(move |_, _| div().text_xs().child(count.to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.active_tag = Some(id);
                            this.select_first_visible(cx);
                            cx.notify();
                        }))
                }),
            )))
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
                                .child(h_flex().gap_1().children(self.file_tags(file, cx)))
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = Some(index);
                    cx.notify();
                })),
        )
    }

    fn file_tags(&self, file: &DemoFile, cx: &App) -> Vec<Tag> {
        self.library
            .tags
            .iter()
            .filter(|tag| file.tags.contains(&tag.id()))
            .map(|tag| {
                let color = tag_color(tag.name(), cx);
                let foreground = if tag.name() == "To review" {
                    cx.theme().warning_foreground
                } else {
                    cx.theme().primary_foreground
                };
                Tag::custom(color, foreground, color)
                    .small()
                    .child(tag.name().to_owned())
            })
            .collect()
    }

    fn details(&self, cx: &App) -> impl IntoElement {
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
            .child(
                h_flex()
                    .justify_between()
                    .text_sm()
                    .font_medium()
                    .child("File details")
                    .child(IconName::Info),
            );
        let Some(file) = self
            .selected
            .and_then(|index| self.library.files.get(index))
        else {
            return panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a file to take a closer look."),
            );
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
                v_flex().gap_3().child(section_label("TAGS", cx)).child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .children(self.file_tags(file, cx)),
                ),
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
                            .child(format!("Learning library / {}", file.info.name())),
                    ),
            )
    }
}

impl Render for FileManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value();
        let visible = self.library.visible_files(self.active_tag, &query);
        let title = self
            .library
            .tags
            .iter()
            .find(|tag| Some(tag.id()) == self.active_tag)
            .map_or("All files", controllers::TagInfo::name)
            .to_owned();
        let columns = if window.viewport_size().width < px(1240.) {
            2
        } else {
            3
        };
        v_flex().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(h_flex().flex_1().min_h_0()
                .child(self.sidebar(cx))
                .child(v_flex().flex_1().min_w_0().h_full()
                    .child(h_flex().h(px(68.)).flex_shrink_0().px_6().gap_3().border_b_1().border_color(cx.theme().border)
                        .child(Icon::new(IconName::Folder).text_color(cx.theme().muted_foreground))
                        .child(div().text_sm().child("Learning library"))
                        .child(div().flex_1())
                        .child(div().w(px(268.)).child(Input::new(&self.search).prefix(IconName::Search).cleanable(true)))
                        .child(Button::new("theme-toggle")
                            .primary()
                            .icon(if cx.theme().is_dark() { IconName::Sun } else { IconName::Moon })
                            .accessibility_label("Dark mode")
                            .toggled(cx.theme().is_dark())
                            .tooltip(if cx.theme().is_dark() { "Switch to light mode" } else { "Switch to dark mode" })
                            .on_click(cx.listener(|_, _, window, cx| {
                                let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
                                Theme::change(mode, Some(window), cx);
                                configure_theme(cx);
                                cx.notify();
                            }))))
                    .child(v_flex().p_6().gap_5().flex_shrink_0()
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
                .child("6 sample files · Temporary backend library · No changes to your files"))
    }
}

fn tag_color(name: &str, cx: &App) -> Hsla {
    match name {
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
