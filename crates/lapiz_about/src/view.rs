use std::sync::Arc;

use anyhow::Result;
use iced_core::{Element, Length, Size, Theme, widget::Void, window};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_widgets::{
    button, collapsible, column, flex::Flex, fluent_builder::WhenSome as _, label, panel, row,
    scrollable, tabbed_view, text_input, title_bar,
};

use crate::{ABOUT_VIEW_ID, CrateEntry, GPL_LICENSE_TEXT, MIT_LICENSE_TEXT, ThirdPartyLicenses};

/// Active tab selection in [`AboutView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AboutTab {
    #[default]
    About,
    ThirdParty,
}

impl AboutTab {
    fn index(self) -> usize {
        match self {
            AboutTab::About => 0,
            AboutTab::ThirdParty => 1,
        }
    }

    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(AboutTab::About),
            1 => Some(AboutTab::ThirdParty),
            _ => None,
        }
    }
}

/// Window view displaying application version details, primary project licenses, and a searchable
/// browser for third-party dependency licenses.
///
/// The view is divided into two tabs:
///
/// - **About**: Shows the application version and collapsible full-text viewers for the project's
///   GPL and MIT licenses.
/// - **Third-Party Licenses**: Provides a filterable sidebar of embedded [`CrateEntry`] records
///   paired with a detail pane displaying the selected crate's version, repository link, and license text.
pub struct AboutView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    tab: AboutTab,
    licenses: ThirdPartyLicenses,
    gpl_open: bool,
    mit_open: bool,
    search: String,
    filtered: Vec<usize>,
    selected: Option<usize>,
}

/// UI interaction and window control messages handled by [`AboutView`].
#[derive(Clone)]
pub enum AboutMessage {
    SelectTab(usize),
    ToggleGpl,
    ToggleMit,
    SearchChanged(String),
    SelectCrate(usize),
    Close,
    Maximize,
    Minimize,
    Drag,
}

impl WindowView for AboutView {
    type Message = AboutMessage;

    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new(ABOUT_VIEW_ID)
    }

    fn boot(
        _params: Option<Self::BootParams>,
        _globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let licenses = ThirdPartyLicenses::parse_embedded();
        let filtered = (0..licenses.crates.len()).collect();
        let (window, open) = open(window::Settings {
            decorations: false,
            size: Size {
                width: 760.0,
                height: 540.0,
            },
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });
        Ok((
            Self {
                window,
                windows: [window].into(),
                tab: AboutTab::default(),
                licenses,
                gpl_open: true,
                mit_open: false,
                search: String::new(),
                filtered,
                selected: None,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let titlebar = title_bar(label(t!("about_title")).window_title())
            .on_close(AboutMessage::Close)
            .on_maximize(AboutMessage::Maximize)
            .on_minimize(AboutMessage::Minimize)
            .on_drag(AboutMessage::Drag);

        let tabs = tabbed_view(
            [
                (t!("about_tab"), self.view_about()),
                (t!("third_party_licenses_tab"), self.view_third_party()),
            ],
            self.tab.index(),
        )
        .on_select(AboutMessage::SelectTab);

        panel(column![titlebar, tabs])
    }

    fn update(
        &mut self,
        message: Self::Message,
        _globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            AboutMessage::SelectTab(index) => {
                if let Some(tab) = AboutTab::from_index(index) {
                    self.tab = tab;
                }
                Task::none()
            }
            AboutMessage::ToggleGpl => {
                self.gpl_open = !self.gpl_open;
                Task::none()
            }
            AboutMessage::ToggleMit => {
                self.mit_open = !self.mit_open;
                Task::none()
            }
            AboutMessage::SearchChanged(search) => {
                self.search = search;
                self.filter_crates();
                Task::none()
            }
            AboutMessage::SelectCrate(index) => {
                self.selected = Some(index);
                Task::none()
            }
            AboutMessage::Close => close(self.window),
            AboutMessage::Maximize => toggle_maximize(self.window),
            AboutMessage::Minimize => minimize(self.window, true),
            AboutMessage::Drag => drag(self.window),
        }
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}

impl AboutView {
    fn filter_crates(&mut self) {
        let query = self.search.to_lowercase();
        self.filtered = self
            .licenses
            .crates
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
    }

    fn view_about(&self) -> Element<'_, AboutMessage, Theme, Renderer> {
        let header = column![
            label("Lapiz").strong().size(22),
            label(t!("about_version", version = env!("CARGO_PKG_VERSION"))).muted(),
            label(t!("about_license_line")).muted()
        ]
        .gap(4);

        let gpl = collapsible(
            label(t!("gpl_license_header")).strong(),
            panel(label(GPL_LICENSE_TEXT).width(Length::Fill))
                .padding([8, 12])
                .width(Length::Fill),
            self.gpl_open,
        )
        .on_toggle(AboutMessage::ToggleGpl);
        let mit = collapsible(
            label(t!("mit_license_header")).strong(),
            panel(label(MIT_LICENSE_TEXT).width(Length::Fill))
                .padding([8, 12])
                .width(Length::Fill),
            self.mit_open,
        )
        .on_toggle(AboutMessage::ToggleMit);

        scrollable(
            column![header, gpl, mit]
                .gap(10)
                .padding(12)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_third_party(&self) -> Element<'_, AboutMessage, Theme, Renderer> {
        let items = if self.filtered.is_empty() {
            vec![label(t!("about_no_results")).muted().into()]
        } else {
            self.filtered
                .iter()
                .map(|&index| {
                    let entry = &self.licenses.crates[index];
                    button(label(entry.name.clone()))
                        .width(Length::Fill)
                        .activated(self.selected == Some(index))
                        .on_press(AboutMessage::SelectCrate(index))
                        .into()
                })
                .collect::<Vec<_>>()
        };

        let sidebar = panel(
            column![
                text_input(&t!("about_search_placeholder"), &self.search)
                    .on_input(AboutMessage::SearchChanged)
                    .width(Length::Fill),
                scrollable(Flex::column(items).gap(2).width(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .gap(6),
        )
        .padding(8)
        .width(240);

        let details = match self.selected.map(|index| &self.licenses.crates[index]) {
            Some(entry) => self.view_crate(entry),
            None => Void.into(),
        };

        row![sidebar, details]
            .gap(8)
            .padding(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_crate(&self, entry: &CrateEntry) -> Element<'_, AboutMessage, Theme, Renderer> {
        let license = self.licenses.licenses.get(entry.license);
        let license_name = license
            .map(|license| license.name.clone())
            .unwrap_or_default();
        let license_text = license
            .map(|license| license.text.clone())
            .unwrap_or_default();

        let header = column![
            label(format!("{} {}", entry.name, entry.version))
                .strong()
                .size(14),
            label(license_name).muted()
        ]
        .gap(2)
        .when_some(entry.repository.clone(), |column, url| {
            column.push(label(url).muted())
        });

        let text = panel(
            scrollable(label(license_text).width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill);

        column![header, text]
            .gap(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
