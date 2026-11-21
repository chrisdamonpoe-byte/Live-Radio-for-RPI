//! Live Radio — a desktop wrapper for iHeart's public live station streams.

mod api;
mod player;
mod store;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, FontId, Frame, Id, Key, Layout, Margin, Modifiers,
    Rect, RichText, Sense, Stroke, Vec2,
};

use api::{Genre, Station};
use player::Player;
use store::Store;
use widgets::*;

const NAV_W: f32 = 92.0;
const PLAYER_H: f32 = 90.0;
const TABS_H: f32 = 60.0;
/// Below this width (or whenever the window is taller than wide) the app uses
/// the portrait layout: tabs along the bottom and a compact player.
const PORTRAIT_MAX_W: f32 = 760.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Home,
    Radio,
    Favorites,
    Search,
}

enum Msg {
    Boot(Result<api::Bootstrap, String>),
    Genre {
        id: u64,
        result: Result<Vec<Station>, String>,
    },
    Meta {
        generation: u64,
        title: Option<String>,
    },
}

enum Action {
    Tab(Tab),
    Genre(Option<u64>),
    Play(Station),
    Favorite(Station),
    SetVolume(f32, bool),
    ToggleMute,
    Toggle,
    Retry,
}

struct Shot {
    path: PathBuf,
    frames: u32,
    total: u32,
    requested: bool,
    waited: u32,
}

struct App {
    tab: Tab,
    genres: Vec<Genre>,
    all: Vec<Station>,
    all_ids: Vec<u64>,
    index: HashMap<u64, usize>,
    genre_ids: HashMap<u64, Vec<u64>>,
    loading_genres: HashSet<u64>,
    radio_genre: Option<u64>,
    loading: bool,
    error: Option<String>,
    search: String,
    focus_search: bool,
    player: Player,
    store: Store,
    art: ArtCache,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    toast: Option<(String, Instant)>,
    shot: Option<Shot>,
    meta_gen: u64,
    meta_at: Option<Instant>,
    meta_pending: bool,
    portrait: bool,
    sized: bool,
    hosted_fullscreen: bool,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        shot_path: Option<PathBuf>,
        tab: Tab,
        search: String,
    ) -> Self {
        let ctx = &cc.egui_ctx;

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = BG;
        visuals.faint_bg_color = CARD;
        visuals.extreme_bg_color = SIDEBAR;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = ACCENT;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, TRACK);
        visuals.widgets.inactive.bg_fill = CARD;
        visuals.widgets.inactive.weak_bg_fill = CARD;
        visuals.widgets.hovered.bg_fill = CARD_HI;
        visuals.widgets.active.bg_fill = CARD_HI;
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(12.0, 6.0);
        ctx.set_style(style);

        let (tx, rx) = mpsc::channel();
        let store = Store::load();
        let volume = store.state.volume;

        let mut app = Self {
            tab,
            genres: Vec::new(),
            all: Vec::new(),
            all_ids: Vec::new(),
            index: HashMap::new(),
            genre_ids: HashMap::new(),
            loading_genres: HashSet::new(),
            radio_genre: None,
            loading: true,
            error: None,
            search,
            focus_search: false,
            player: Player::new(volume),
            store,
            art: ArtCache::new(),
            tx,
            rx,
            toast: None,
            shot: shot_path.map(|path| Shot {
                path,
                frames: 0,
                total: 0,
                requested: false,
                waited: 0,
            }),
            meta_gen: 0,
            meta_at: None,
            meta_pending: false,
            portrait: false,
            sized: false,
            hosted_fullscreen: false,
        };

        let saved: Vec<Station> = app
            .store
            .state
            .favorites
            .iter()
            .chain(app.store.state.recent.iter())
            .cloned()
            .collect();
        app.merge(saved);
        app.spawn_boot();
        app
    }

    fn spawn_boot(&mut self) {
        self.loading = true;
        self.error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Boot(api::bootstrap()));
        });
    }

    fn open_genre(&mut self, genre: Option<u64>) {
        self.tab = Tab::Radio;
        self.radio_genre = genre;
        if let Some(id) = genre {
            if !self.genre_ids.contains_key(&id) && !self.loading_genres.contains(&id) {
                self.loading_genres.insert(id);
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let result = api::fetch_stations(Some(id), 60);
                    let _ = tx.send(Msg::Genre { id, result });
                });
            }
        }
    }

    fn merge(&mut self, stations: Vec<Station>) {
        for fresh in stations {
            if let Some(&i) = self.index.get(&fresh.id) {
                let old = &mut self.all[i];
                if !fresh.hls.is_empty() {
                    old.hls = fresh.hls;
                }
                if !fresh.shoutcast.is_empty() {
                    old.shoutcast = fresh.shoutcast;
                }
                if !fresh.name.is_empty() {
                    old.name = fresh.name;
                }
                if !fresh.call.is_empty() {
                    old.call = fresh.call;
                }
                if !fresh.desc.is_empty() {
                    old.desc = fresh.desc;
                }
                if !fresh.freq.is_empty() {
                    old.freq = fresh.freq;
                }
                if !fresh.genre.is_empty() {
                    old.genre = fresh.genre;
                }
                if !fresh.market.is_empty() {
                    old.market = fresh.market;
                }
                if fresh.cume > 0 {
                    old.cume = fresh.cume;
                }
            } else {
                self.index.insert(fresh.id, self.all.len());
                self.all_ids.push(fresh.id);
                self.all.push(fresh);
            }
        }
    }

    fn start_meta(&mut self) {
        let Some(station) = self.player.station.clone() else {
            return;
        };
        self.meta_gen = self.meta_gen.wrapping_add(1);
        let generation = self.meta_gen;
        self.meta_pending = true;
        self.meta_at = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let title = api::probe_metadata(&station);
            let _ = tx.send(Msg::Meta {
                generation,
                title,
            });
        });
    }

    fn set_toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Tab(tab) => {
                self.tab = tab;
                if tab == Tab::Search {
                    self.focus_search = true;
                }
            }
            Action::Genre(genre) => self.open_genre(genre),
            Action::Play(station) => {
                if self.player.station_id() == Some(station.id) && self.player.active() {
                    self.player.toggle();
                } else {
                    self.player.play(&station);
                    self.store.remember(&station);
                    self.start_meta();
                }
            }
            Action::Favorite(station) => {
                let added = self.store.toggle_favorite(&station);
                self.set_toast(if added {
                    "Added to favorites"
                } else {
                    "Removed from favorites"
                });
            }
            Action::SetVolume(value, persist) => {
                self.player.set_volume(value);
                self.store.set_volume_in_memory(value);
                if persist {
                    self.store.save_volume();
                }
            }
            Action::ToggleMute => {
                self.player.toggle_mute();
                let volume = self.player.volume;
                self.store.set_volume_in_memory(volume);
                self.store.save_volume();
            }
            Action::Toggle => {
                let was_playing = self.player.playing();
                self.player.toggle();
                if !was_playing && self.player.playing() {
                    self.start_meta();
                }
            }
            Action::Retry => {
                if !self.loading {
                    self.spawn_boot();
                }
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Boot(result) => {
                    self.loading = false;
                    match result {
                        Ok(boot) => {
                            self.error = None;
                            self.genres = boot.genres;
                            self.merge(boot.stations);
                            self.store.refresh_saved(&self.all);
                            if self.shot.is_some() {
                                if self.store.state.recent.is_empty() {
                                    self.store.state.recent =
                                        self.all.iter().take(4).cloned().collect();
                                }
                                if self.player.station.is_none() {
                                    if let Some(first) = self.all.first().cloned() {
                                        self.player.preview(&first);
                                    }
                                }
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Msg::Genre { id, result } => {
                    self.loading_genres.remove(&id);
                    match result {
                        Ok(list) => {
                            self.genre_ids
                                .insert(id, list.iter().map(|station| station.id).collect());
                            self.merge(list);
                        }
                        Err(error) => self.set_toast(error),
                    }
                }
                Msg::Meta {
                    generation,
                    title,
                } => {
                    if generation == self.meta_gen {
                        self.meta_pending = false;
                        self.meta_at = Some(Instant::now());
                        if let Some(title) = title {
                            if !title.is_empty() {
                                self.player.now_playing = Some(title);
                            }
                        }
                    }
                }
            }
        }

        if let Some((_, at)) = &self.toast {
            if at.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }
    }

    // ---------------------------------------------------------------------
    // Layout
    // ---------------------------------------------------------------------

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(58.0, 58.0), Sense::hover());
            let logo = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter().rect_filled(logo, 11.0, ACCENT);
            icon(
                ui.painter(),
                logo.shrink(12.0),
                Icon::Radio,
                Color32::WHITE,
            );
            ui.add_space(7.0);
            ui.label(RichText::new("LIVE").size(10.0).strong().color(TEXT));
            ui.label(RichText::new("RADIO").size(9.0).color(MUTED));
        });

        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if nav_item(ui, Icon::Home, "Home", self.tab == Tab::Home).clicked() {
                self.tab = Tab::Home;
            }
            if nav_item(ui, Icon::Radio, "Radio", self.tab == Tab::Radio).clicked() {
                self.tab = Tab::Radio;
            }
            if nav_item(ui, Icon::Heart, "Favorites", self.tab == Tab::Favorites).clicked() {
                self.tab = Tab::Favorites;
            }
            if nav_item(ui, Icon::Search, "Search", self.tab == Tab::Search).clicked() {
                self.tab = Tab::Search;
                self.focus_search = true;
            }
        });

        let panel = ui.max_rect();
        ui.painter().line_segment(
            [pos2(panel.right(), panel.top()), pos2(panel.right(), panel.bottom())],
            Stroke::new(1.0_f32, Color32::from_black_alpha(80)),
        );

        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(env!("CARGO_PKG_VERSION")).size(9.5).color(FAINT));
            ui.add_space(6.0);
            if self.loading {
                ui.spinner();
            } else if self.error.is_some() {
                ui.label(RichText::new("offline").size(9.5).color(LIVE));
            }
        });
    }

    /// Bottom tab bar used by the portrait layout.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [pos2(rect.left(), rect.top()), pos2(rect.right(), rect.top())],
            Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
        );
        let tabs = [
            (Icon::Home, "Home", Tab::Home),
            (Icon::Radio, "Radio", Tab::Radio),
            (Icon::Heart, "Favorites", Tab::Favorites),
            (Icon::Search, "Search", Tab::Search),
        ];
        let width = rect.width() / tabs.len() as f32;
        for (i, (kind, label, tab)) in tabs.into_iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(rect.left() + width * i as f32, rect.top() + 2.0),
                vec2(width, TABS_H - 2.0),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(cell)
                    .layout(Layout::top_down(Align::Center)),
            );
            if nav_item(&mut child, kind, label, self.tab == tab).clicked() {
                self.tab = tab;
                if tab == Tab::Search {
                    self.focus_search = true;
                }
            }
        }
    }

    /// Genre chips: wrapped on wide windows, one swipeable row in portrait.
    fn genre_chips(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, limit: usize) {
        let genres = self.genres.clone();
        let selected = self.radio_genre;
        let mut draw = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            if chip(ui, "All stations", selected.is_none()).clicked() {
                actions.push(Action::Genre(None));
            }
            for genre in genres.iter().take(limit) {
                if chip(ui, &genre.name, selected == Some(genre.id)).clicked() {
                    actions.push(Action::Genre(Some(genre.id)));
                }
            }
        };
        if self.portrait {
            egui::ScrollArea::horizontal()
                .id_salt("genre-chips")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| ui.horizontal(|ui| draw(ui)));
        } else {
            ui.horizontal_wrapped(|ui| draw(ui));
        }
    }

    fn page_header(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).size(27.0).strong().color(TEXT));
                ui.add_space(1.0);
                ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
            });
            if self.player.playing() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    on_air(ui, ui.ctx().input(|i| i.time));
                });
            }
        });
        ui.add_space(16.0);
    }

    fn loading_view(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(110.0);
            ui.spinner();
            ui.add_space(10.0);
            ui.label(RichText::new("Tuning in…").size(14.0).color(MUTED));
            ui.label(
                RichText::new("Loading live stations")
                    .size(11.5)
                    .color(FAINT),
            );
        });
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let mut actions: Vec<Action> = Vec::new();

        if let Some(error) = self.error.clone() {
            Frame::none()
                .fill(ACCENT_SOFT)
                .rounding(8.0)
                .inner_margin(Margin::symmetric(14.0, 10.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Stations are offline")
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(RichText::new(error).size(11.0).color(MUTED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Try again").size(11.5).color(TEXT),
                                    )
                                    .fill(CARD)
                                    .rounding(14.0),
                                )
                                .clicked()
                            {
                                actions.push(Action::Retry);
                            }
                        });
                    });
                });
            ui.add_space(14.0);
        }

        match self.tab {
            Tab::Home => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.home(ui, &mut actions));
            }
            Tab::Radio => self.radio(ui, &mut actions),
            Tab::Favorites => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.favorites(ui, &mut actions));
            }
            Tab::Search => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.search_view(ui, &mut actions));
            }
        }

        for action in actions {
            self.apply(action);
        }
    }

    fn home(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let subtitle = if self.all.is_empty() {
            "Live radio, always on".to_string()
        } else {
            format!("Live radio, always on · {} stations", self.all.len())
        };
        self.page_header(ui, "Home", &subtitle);

        if self.loading && self.all.is_empty() {
            self.loading_view(ui);
            return;
        }

        self.genre_chips(ui, actions, if self.portrait { usize::MAX } else { 10 });
        ui.add_space(24.0);

        let recents = self.store.state.recent.clone();
        if !recents.is_empty() {
            section_header(ui, "Recently played", None);
            let current = self.player.station_id();
            let playing = self.player.playing();
            egui::ScrollArea::horizontal()
                .id_salt("recently-played")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        for station in recents.iter().take(6) {
                            if mini_card(
                                ui,
                                &mut self.art,
                                station,
                                102.0,
                                playing && current == Some(station.id),
                            ) {
                                actions.push(Action::Play(station.clone()));
                            }
                        }
                    });
                });
            ui.add_space(26.0);
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Top stations").size(18.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("Browse radio").size(12.0).color(MUTED))
                            .frame(false),
                    )
                    .clicked()
                {
                    actions.push(Action::Tab(Tab::Radio));
                }
            });
        });
        ui.add_space(10.0);

        let ids = self.all_ids.clone();
        let favorites: Vec<u64> = self
            .store
            .state
            .favorites
            .iter()
            .map(|station| station.id)
            .collect();
        let current = if self.player.playing() {
            self.player.station_id()
        } else {
            None
        };
        let shown = station_grid(
            ui,
            &mut self.art,
            &self.all,
            &self.index,
            &ids,
            30,
            current,
            &favorites,
            actions,
        );
        if self.all.len() > shown {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Showing {} of {} stations — browse by genre for more",
                    shown,
                    self.all.len()
                ))
                .size(11.0)
                .color(FAINT),
            );
        }
        ui.add_space(20.0);
    }

    fn radio(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(ui, "Radio", "Browse live stations by genre");

        let genres = self.genres.clone();
        let selected = self.radio_genre;
        let ids: Vec<u64> = match selected {
            Some(id) => self.genre_ids.get(&id).cloned().unwrap_or_default(),
            None => self.all_ids.clone(),
        };
        let title = selected
            .and_then(|id| {
                genres
                    .iter()
                    .find(|genre| genre.id == id)
                    .map(|genre| genre.name.clone())
            })
            .unwrap_or_else(|| "All stations".to_string());
        let loading = selected
            .map(|id| self.loading_genres.contains(&id))
            .unwrap_or(false);
        let favorites: Vec<u64> = self
            .store
            .state
            .favorites
            .iter()
            .map(|station| station.id)
            .collect();
        let current = if self.player.playing() {
            self.player.station_id()
        } else {
            None
        };
        if self.portrait {
            self.genre_chips(ui, actions, usize::MAX);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&title).size(17.0).strong().color(TEXT));
                if loading {
                    ui.spinner();
                } else {
                    ui.label(
                        RichText::new(format!("{} stations", ids.len()))
                            .size(11.0)
                            .color(FAINT),
                    );
                }
            });
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if ids.is_empty() && loading {
                        ui.add_space(40.0);
                        ui.spinner();
                    } else {
                        station_grid(
                            ui,
                            &mut self.art,
                            &self.all,
                            &self.index,
                            &ids,
                            60,
                            current,
                            &favorites,
                            actions,
                        );
                    }
                });
            return;
        }

        let body_height = ui.available_height().max(200.0);

        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.allocate_ui_with_layout(
                vec2(186.0, body_height),
                Layout::top_down(Align::LEFT),
                |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            if genre_row(ui, "All stations", selected.is_none()).clicked() {
                                actions.push(Action::Genre(None));
                            }
                            for genre in &genres {
                                if genre_row(ui, &genre.name, selected == Some(genre.id)).clicked()
                                {
                                    actions.push(Action::Genre(Some(genre.id)));
                                }
                            }
                        });
                },
            );

            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), body_height),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).size(17.0).strong().color(TEXT));
                        if loading {
                            ui.spinner();
                        } else if selected.is_some() && ids.is_empty() {
                            ui.label(RichText::new("No stations").size(11.0).color(FAINT));
                        } else {
                            ui.label(
                                RichText::new(format!("{} stations", ids.len()))
                                    .size(11.0)
                                    .color(FAINT),
                            );
                        }
                    });
                    ui.add_space(12.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if ids.is_empty() && loading {
                                ui.add_space(40.0);
                                ui.spinner();
                            } else {
                                station_grid(
                                    ui,
                                    &mut self.art,
                                    &self.all,
                                    &self.index,
                                    &ids,
                                    60,
                                    current,
                                    &favorites,
                                    actions,
                                );
                            }
                        });
                },
            );
        });
    }

    fn favorites(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(ui, "Favorites", "The stations you saved");

        let saved = self.store.state.favorites.clone();
        if saved.is_empty() {
            if empty_state(
                ui,
                Icon::Heart,
                "No favorites yet",
                "Tap the heart on a station while you listen, and it will wait for you here.",
                "Browse stations",
            ) {
                actions.push(Action::Tab(Tab::Radio));
            }
            return;
        }

        let ids: Vec<u64> = saved.iter().map(|station| station.id).collect();
        let favorites = ids.clone();
        let current = if self.player.playing() {
            self.player.station_id()
        } else {
            None
        };
        station_grid(
            ui,
            &mut self.art,
            &self.all,
            &self.index,
            &ids,
            60,
            current,
            &favorites,
            actions,
        );
        ui.add_space(20.0);
    }

    fn search_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        self.page_header(ui, "Search", "Find a station by name, call sign, city or genre");

        let response = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search stations…")
                .desired_width(ui.available_width().min(400.0))
                .font(FontId::proportional(14.0)),
        );
        if self.focus_search {
            response.request_focus();
            self.focus_search = false;
        }
        ui.add_space(12.0);

        let query = self.search.trim().to_lowercase();
        if query.is_empty() {
            ui.label(
                RichText::new(format!(
                    "Search across the {} stations already loaded. Use Radio to browse by genre.",
                    self.all.len()
                ))
                .size(12.0)
                .color(FAINT),
            );
            return;
        }

        let matches: Vec<&Station> = self
            .all
            .iter()
            .filter(|station| {
                station.name.to_lowercase().contains(&query)
                    || station.call.to_lowercase().contains(&query)
                    || station.market.to_lowercase().contains(&query)
                    || station.genre.to_lowercase().contains(&query)
                    || station.desc.to_lowercase().contains(&query)
                    || station.freq.to_lowercase().contains(&query)
            })
            .collect();

        ui.label(
            RichText::new(format!(
                "{} {}",
                matches.len(),
                if matches.len() == 1 {
                    "station"
                } else {
                    "stations"
                }
            ))
            .size(11.5)
            .color(FAINT),
        );
        ui.add_space(12.0);

        if matches.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Nothing matched that search")
                        .size(15.0)
                        .color(TEXT),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Try a call sign such as Z100, or a city name.")
                        .size(12.0)
                        .color(MUTED),
                );
            });
            return;
        }

        let ids: Vec<u64> = matches.iter().take(120).map(|station| station.id).collect();
        let favorites: Vec<u64> = self
            .store
            .state
            .favorites
            .iter()
            .map(|station| station.id)
            .collect();
        let current = if self.player.playing() {
            self.player.station_id()
        } else {
            None
        };
        station_grid(
            ui,
            &mut self.art,
            &self.all,
            &self.index,
            &ids,
            60,
            current,
            &favorites,
            actions,
        );
        ui.add_space(20.0);
    }

    // ---------------------------------------------------------------------
    // Player bar
    // ---------------------------------------------------------------------

    fn player_bar(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, PLAYER_H), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 0.0, SIDEBAR);
        painter.line_segment(
            [pos2(rect.left(), rect.top()), pos2(rect.right(), rect.top())],
            Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
        );

        let mut actions: Vec<Action> = Vec::new();
        let station = self.player.station.clone();
        let cy = rect.center().y;
        let art_rect = Rect::from_min_size(pos2(rect.left() + 20.0, cy - 28.0), vec2(56.0, 56.0));

        match &station {
            Some(st) => {
                let (texture, _) = self.art.art_for(ui.ctx(), st);
                art_image(&painter, art_rect, texture);
                painter.rect_stroke(art_rect, 8.0, Stroke::new(1.0_f32, Color32::from_white_alpha(18)));
            }
            None => {
                painter.rect_filled(art_rect, 8.0, CARD);
                icon(&painter, art_rect.shrink(18.0), Icon::Radio, FAINT);
            }
        }

        let compact = self.portrait;
        let text_x = art_rect.right() + 14.0;
        let text_max = if compact {
            // Leave room for the mute + play buttons on the right.
            (rect.right() - 20.0 - 48.0 - 52.0 - text_x).max(80.0)
        } else {
            (rect.center().x - 84.0 - text_x).clamp(150.0, 420.0)
        };

        if let Some(st) = &station {
            let favorite_here = self.store.is_favorite(st.id);
            let name_font = FontId::proportional(15.0);
            let shown = elide(&st.name, text_max - 28.0, 15.0);
            let galley = painter.layout_no_wrap(shown.clone(), name_font.clone(), TEXT);
            painter.text(
                pos2(text_x, cy - 17.0),
                Align2::LEFT_CENTER,
                shown,
                name_font,
                TEXT,
            );

            let heart_center = pos2(text_x + galley.size().x + 15.0, cy - 17.0);
            let heart_rect = Rect::from_center_size(heart_center, Vec2::splat(22.0));
            let heart_resp = ui.interact(
                heart_rect,
                ui.id().with("player-favorite"),
                Sense::click(),
            );
            let heart_hover = ui.ctx().animate_bool(heart_resp.id, heart_resp.hovered());
            icon(
                &painter,
                heart_rect.shrink(4.0),
                Icon::Heart,
                if favorite_here {
                    ACCENT_HI
                } else if heart_hover > 0.0 {
                    TEXT
                } else {
                    MUTED
                },
            );
            if heart_resp.clicked() {
                actions.push(Action::Favorite(st.clone()));
            }

            line(
                &painter,
                pos2(text_x, cy + 1.0),
                &st.subtitle(),
                11.5,
                MUTED,
                text_max,
            );

            let (status, color) = if self.player.failed {
                (self.player.status.clone(), LIVE)
            } else if self.player.paused {
                ("Paused".to_string(), MUTED)
            } else if self.player.playing() || self.player.active() {
                (
                    self.player
                        .now_playing
                        .clone()
                        .unwrap_or_else(|| "Live stream".to_string()),
                    TEXT,
                )
            } else {
                (self.player.status.clone(), MUTED)
            };
            let status_x = if self.player.playing() {
                painter.circle_filled(pos2(text_x + 3.0, cy + 19.0), 3.0, LIVE);
                text_x + 12.0
            } else {
                text_x
            };
            line(
                &painter,
                pos2(status_x, cy + 19.0),
                &status,
                11.0,
                color,
                text_max - (status_x - text_x),
            );
        } else {
            painter.text(
                pos2(text_x, cy - 17.0),
                Align2::LEFT_CENTER,
                "Nothing playing",
                FontId::proportional(15.0),
                TEXT,
            );
            painter.text(
                pos2(text_x, cy + 1.0),
                Align2::LEFT_CENTER,
                "Pick a station to start listening",
                FontId::proportional(11.5),
                MUTED,
            );
            painter.text(
                pos2(text_x, cy + 19.0),
                Align2::LEFT_CENTER,
                &self.player.status,
                FontId::proportional(11.0),
                FAINT,
            );
        }

        // Centre transport button.
        let button_x = if compact {
            rect.right() - 20.0 - 24.0
        } else {
            rect.center().x
        };
        let button_rect = Rect::from_center_size(pos2(button_x, cy - 1.0), Vec2::splat(48.0));
        let button_resp = ui.interact(
            button_rect,
            ui.id().with("player-toggle"),
            Sense::click(),
        );
        let enabled = station.is_some();
        let hover = ui.ctx().animate_bool(button_resp.id, button_resp.hovered());
        let fill = if !enabled {
            CARD
        } else if hover > 0.0 {
            lerp_color(ACCENT, ACCENT_HI, hover)
        } else {
            ACCENT
        };
        painter.circle_filled(button_rect.center(), 23.0, fill);
        icon(
            &painter,
            button_rect.shrink(15.0),
            if self.player.playing() {
                Icon::Pause
            } else {
                Icon::Play
            },
            if enabled { Color32::WHITE } else { FAINT },
        );
        if button_resp.clicked() {
            if enabled {
                actions.push(Action::Toggle);
            } else {
                actions.push(Action::Tab(Tab::Radio));
            }
        }

        if self.player.playing() {
            let elapsed = fmt_elapsed(self.player.elapsed());
            let text = format!("LIVE · {elapsed}");
            painter.text(
                pos2(button_x, cy + 32.0),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(9.0),
                MUTED,
            );
        }

        if compact {
            // Portrait: just a mute toggle beside the play button.
            let volume = self.player.volume;
            let mute_rect =
                Rect::from_center_size(pos2(button_rect.left() - 26.0, cy - 1.0), Vec2::splat(34.0));
            let mute_resp = ui.interact(mute_rect, ui.id().with("mute"), Sense::click());
            icon(
                &painter,
                mute_rect.shrink(8.0),
                if volume <= 0.001 { Icon::Mute } else { Icon::Volume },
                if volume <= 0.001 { ACCENT_HI } else { MUTED },
            );
            if mute_resp.clicked() {
                actions.push(Action::ToggleMute);
            }
            for action in actions {
                self.apply(action);
            }
            return;
        }

        // Volume on the right.
        let right = rect.right() - 20.0;
        let bar_width = 104.0;
        let bar_left = right - 30.0 - bar_width - 8.0;
        let bar_rect = Rect::from_min_size(pos2(bar_left, cy - 9.0), vec2(bar_width, 18.0));
        let volume_resp = ui.interact(bar_rect, ui.id().with("volume"), Sense::click_and_drag());
        if volume_resp.dragged() || volume_resp.clicked() {
            if let Some(pos) = volume_resp.interact_pointer_pos() {
                let value = ((pos.x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
                actions.push(Action::SetVolume(
                    value,
                    volume_resp.drag_stopped() || volume_resp.clicked(),
                ));
            }
        }
        let volume = self.player.volume;
        painter.rect_filled(
            Rect::from_min_size(pos2(bar_left, cy - 2.0), vec2(bar_width, 4.0)),
            2.0,
            TRACK,
        );
        painter.rect_filled(
            Rect::from_min_size(
                pos2(bar_left, cy - 2.0),
                vec2(bar_width * volume, 4.0),
            ),
            2.0,
            Color32::from_white_alpha(210),
        );
        painter.circle_filled(
            pos2(bar_left + bar_width * volume, cy),
            5.0,
            Color32::WHITE,
        );

        let mute_rect = Rect::from_center_size(pos2(bar_left - 20.0, cy), Vec2::splat(26.0));
        let mute_resp = ui.interact(mute_rect, ui.id().with("mute"), Sense::click());
        let mute_hover = ui.ctx().animate_bool(mute_resp.id, mute_resp.hovered());
        icon(
            &painter,
            mute_rect.shrink(5.0),
            if volume <= 0.001 {
                Icon::Mute
            } else {
                Icon::Volume
            },
            if volume <= 0.001 {
                ACCENT_HI
            } else if mute_hover > 0.0 {
                TEXT
            } else {
                MUTED
            },
        );
        if mute_resp.clicked() {
            actions.push(Action::ToggleMute);
        }

        painter.text(
            pos2(right, cy),
            Align2::RIGHT_CENTER,
            format!("{:.0}", volume * 100.0),
            FontId::proportional(10.0),
            FAINT,
        );

        for action in actions {
            self.apply(action);
        }
    }

    fn toast(&self, ctx: &egui::Context) {
        let Some((text, _)) = &self.toast else {
            return;
        };
        egui::Area::new(Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-18.0, -(PLAYER_H + 16.0)))
            .show(ctx, |ui| {
                Frame::none()
                    .fill(CARD)
                    .rounding(8.0)
                    .inner_margin(Margin::symmetric(14.0, 9.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(12.0).color(TEXT));
                    });
            });
    }

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() {
            return;
        }

        let mut captured: Option<egui::ColorImage> = None;
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    captured = Some((**image).clone());
                }
            }
        });

        if let Some(image) = captured {
            let path = self.shot.as_ref().map(|shot| shot.path.clone()).unwrap();
            match save_png(&path, &image) {
                Ok(()) => println!("saved screenshot to {}", path.display()),
                Err(error) => eprintln!("could not save screenshot: {error}"),
            }
            self.shot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let ready = !self.all.is_empty() || self.error.is_some();
        let Some(shot) = self.shot.as_mut() else {
            return;
        };
        shot.total = shot.total.saturating_add(1);
        if ready {
            shot.frames = shot.frames.saturating_add(1);
        }

        if shot.frames >= 120 && !shot.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            shot.requested = true;
        }

        if shot.requested {
            shot.waited = shot.waited.saturating_add(1);
        }

        if shot.total > 1800 || shot.waited > 240 {
            eprintln!("screenshot timed out");
            self.shot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.player.poll();

        if self.player.active()
            && !self.player.paused
            && !self.meta_pending
            && self
                .meta_at
                .map(|at| at.elapsed() >= Duration::from_secs(30))
                .unwrap_or(true)
        {
            self.start_meta();
        }

        if ctx.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::F)) {
            self.tab = Tab::Search;
            self.focus_search = true;
        }
        if !ctx.wants_keyboard_input() && ctx.input(|input| input.key_pressed(Key::Space)) {
            self.apply(Action::Toggle);
        }

        // On a portrait screen (e.g. a rotated Pi display) open full screen-size.
        if !self.sized && !self.hosted_fullscreen {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                self.sized = true;
                if monitor.y > monitor.x && self.shot.is_none() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                }
            }
        }

        if self.hosted_fullscreen {
            egui::TopBottomPanel::top("return-to-player")
                .resizable(false)
                .frame(Frame::none().fill(SIDEBAR).inner_margin(egui::Margin::same(6.0)))
                .show(ctx, |ui| {
                    if ui.add(egui::Button::new("Return to Player").min_size(egui::vec2(148.0, 44.0))).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
        }

        let screen = ctx.screen_rect();
        self.portrait = screen.width() < screen.height() || screen.width() < PORTRAIT_MAX_W;

        if self.portrait {
            egui::TopBottomPanel::bottom("tabs")
                .exact_height(TABS_H)
                .resizable(false)
                .frame(Frame::none().fill(SIDEBAR))
                .show(ctx, |ui| self.tab_bar(ui));
        }

        egui::TopBottomPanel::bottom("player")
            .exact_height(PLAYER_H)
            .resizable(false)
            .frame(Frame::none().fill(SIDEBAR))
            .show(ctx, |ui| self.player_bar(ui));

        if !self.portrait {
            egui::SidePanel::left("nav")
                .exact_width(NAV_W)
                .resizable(false)
                .frame(
                    Frame::none()
                        .fill(SIDEBAR)
                        .inner_margin(Margin::symmetric(8.0, 0.0)),
                )
                .show(ctx, |ui| self.nav(ui));
        }

        let margin = if self.portrait {
            Margin::symmetric(14.0, 12.0)
        } else {
            Margin::symmetric(26.0, 14.0)
        };
        egui::CentralPanel::default()
            .frame(Frame::none().fill(BG).inner_margin(margin))
            .show(ctx, |ui| self.content(ui));

        self.toast(ctx);
        self.screenshot_tick(ctx);

        if self.player.playing() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.shutdown();
    }
}

// -------------------------------------------------------------------------
// Drawing helpers
// -------------------------------------------------------------------------

fn on_air(ui: &mut egui::Ui, time: f64) {
    let (rect, _) = ui.allocate_exact_size(vec2(78.0, 22.0), Sense::hover());
    let meter = Rect::from_min_size(pos2(rect.left(), rect.top() + 6.0), vec2(14.0, 10.0));
    equalizer(ui.painter(), meter, time, LIVE);
    ui.painter().text(
        pos2(rect.left() + 21.0, rect.center().y),
        Align2::LEFT_CENTER,
        "ON AIR",
        FontId::proportional(9.5),
        LIVE,
    );
}

fn empty_state(
    ui: &mut egui::Ui,
    glyph: Icon,
    title: &str,
    body: &str,
    button: &str,
) -> bool {
    ui.vertical_centered(|ui| {
        ui.add_space(90.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 32.0, CARD);
        icon(ui.painter(), rect.shrink(20.0), glyph, MUTED);
        ui.add_space(14.0);
        ui.label(RichText::new(title).size(16.0).strong().color(TEXT));
        ui.add_space(2.0);
        ui.label(RichText::new(body).size(12.0).color(MUTED));
        ui.add_space(16.0);
        ui.add(
            egui::Button::new(RichText::new(button).size(12.5).color(Color32::WHITE))
                .fill(ACCENT)
                .rounding(18.0),
        )
        .clicked()
    })
    .inner
}

/// Draw a grid of station cards and return how many were drawn.
#[allow(clippy::too_many_arguments)]
fn station_grid(
    ui: &mut egui::Ui,
    art: &mut ArtCache,
    stations: &[Station],
    index: &HashMap<u64, usize>,
    ids: &[u64],
    max: usize,
    current: Option<u64>,
    favorites: &[u64],
    actions: &mut Vec<Action>,
) -> usize {
    let visible: Vec<&Station> = ids
        .iter()
        .take(max)
        .filter_map(|id| index.get(id).map(|&i| &stations[i]))
        .collect();
    if visible.is_empty() {
        return 0;
    }

    let available = ui.available_width();
    let spacing = 16.0;
    let columns = (((available + spacing) / (136.0 + spacing)).floor() as usize).max(2);
    let card_width =
        ((available - spacing * (columns.saturating_sub(1)) as f32) / columns as f32).floor();
    let card_width = card_width.max(96.0);

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(spacing, spacing);
        for row in visible.chunks(columns) {
            ui.horizontal(|ui| {
                for station in row {
                    let hit = station_card(
                        ui,
                        art,
                        station,
                        card_width,
                        current == Some(station.id),
                        favorites.contains(&station.id),
                    );
                    if hit.favorite {
                        actions.push(Action::Favorite((*station).clone()));
                    } else if hit.play {
                        actions.push(Action::Play((*station).clone()));
                    }
                }
            });
        }
    });

    visible.len()
}

fn fmt_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let [width, height] = image.size;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for color in &image.pixels {
        pixels.extend_from_slice(&[color.r(), color.g(), color.b(), 255]);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    image::save_buffer(
        path,
        &pixels,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

fn main() -> eframe::Result<()> {
    let mut shot_path: Option<PathBuf> = None;
    let mut tab = Tab::Home;
    let mut search = String::new();
    let mut size: Option<[f32; 2]> = None;
    let mut fullscreen = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fullscreen" => fullscreen = true,
            "--screenshot" => shot_path = args.next().map(PathBuf::from),
            "--tab" => {
                tab = match args.next().as_deref() {
                    Some("radio") => Tab::Radio,
                    Some("favorites") => Tab::Favorites,
                    Some("search") => Tab::Search,
                    _ => Tab::Home,
                }
            }
            "--search" => search = args.next().unwrap_or_default(),
            "--size" => {
                size = args.next().and_then(|s| {
                    let (w, h) = s.split_once('x')?;
                    Some([w.parse().ok()?, h.parse().ok()?])
                })
            }
            _ => {}
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(size.unwrap_or([1180.0, 780.0]))
        .with_min_inner_size([360.0, 480.0])
        .with_title("Live Radio")
        .with_app_id("live-radio")
        .with_fullscreen(fullscreen);

    if shot_path.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        vsync: true,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };

    eframe::run_native(
        "Live Radio",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, shot_path.clone(), tab, search.clone());
            app.hosted_fullscreen = fullscreen;
            Ok(Box::new(app))
        }),
    )
}

