use std::{
    collections::HashMap,
    env, fs,
    net::Ipv4Addr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Stroke, Vec2,
};
use serde_json::{Value, json};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MessageBoxW, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    },
};

use wiz_tray::{
    Action, CloudConfig, CloudOAuthConfig, ExportDevice, LightConfig, PREFERRED_SCENE, Pilot,
    clear_cloud_session, find_signing_key, has_cloud_session, load_cloud_credentials,
    normalize_mac, parse_cloud_id, parse_cloud_oauth, parse_export_devices, parse_key,
    parse_white_range, perform, perform_cloud, save_cloud_credentials, temperature_options,
    validate_cloud_config,
};

const SCENES: &[(u16, &str)] = &[
    (PREFERRED_SCENE, "Luz de día"),
    (11, "Blanco cálido"),
    (13, "Blanco frío"),
    (15, "Enfoque"),
    (16, "Relajarse"),
    (14, "Luz nocturna"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Local,
    Cloud,
}

impl Backend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cloud => "cloud",
        }
    }

    fn from_value(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(Value::as_str).unwrap_or("local") {
            "local" => Ok(Self::Local),
            "cloud" => Ok(Self::Cloud),
            _ => Err("La conexión guardada debe ser local o WiZ Cloud.".to_string()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Local => "Casa · Wi-Fi local",
            Self::Cloud => "WiZ Cloud · remoto",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Routines,
    Connections,
}

#[derive(Clone)]
struct Preferences {
    backend: Backend,
    device_name: String,
    local_export: String,
    local_ip: String,
    local_mac: String,
    cloud_device_id: String,
    cloud_home_id: String,
    cloud_mac: String,
    cloud_white_range: Option<(u16, u16)>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            backend: Backend::Local,
            device_name: "Mi foco".to_string(),
            local_export: String::new(),
            local_ip: String::new(),
            local_mac: String::new(),
            cloud_device_id: String::new(),
            cloud_home_id: String::new(),
            cloud_mac: String::new(),
            cloud_white_range: None,
        }
    }
}

#[derive(Clone, Default)]
struct Connections {
    local: Option<LightConfig>,
    cloud: Option<CloudConfig>,
}

impl Connections {
    fn has(&self, backend: Backend) -> bool {
        match backend {
            Backend::Local => self.local.is_some(),
            Backend::Cloud => self.cloud.is_some(),
        }
    }

    fn white_range(&self, backend: Backend) -> Option<(u16, u16)> {
        match backend {
            Backend::Local => self.local.as_ref()?.white_range,
            Backend::Cloud => self.cloud.as_ref()?.white_range,
        }
    }

    fn perform(&self, backend: Backend, action: &Action) -> Result<Pilot, String> {
        match backend {
            Backend::Local => self
                .local
                .as_ref()
                .ok_or_else(|| "Configura primero la conexión local.".to_string())
                .and_then(|config| perform(config, action)),
            Backend::Cloud => self
                .cloud
                .as_ref()
                .ok_or_else(|| "Configura primero WiZ Cloud.".to_string())
                .and_then(|config| perform_cloud(config, action)),
        }
    }
}

#[derive(Clone)]
enum TrayAction {
    Open,
    Control(Action),
    ConnectCloud,
    Exit,
}

enum UiEvent {
    Menu(MenuEvent),
    Finished(Backend, Result<Pilot, String>),
}

struct App {
    preferences: Preferences,
    draft: Preferences,
    connections: Connections,
    export_devices: Vec<ExportDevice>,
    cloud_credentials: Option<CloudOAuthConfig>,
    google_client_id: String,
    google_client_secret: String,
    wiz_client_id: String,
    wiz_client_secret: String,
    replace_credentials: bool,
    cloud_minimum: String,
    cloud_maximum: String,
    page: Page,
    setup_tab: Backend,
    pilot: Option<Pilot>,
    temperature: u16,
    timer_minutes: u16,
    timer_due: Option<Instant>,
    status: String,
    last_error: Option<String>,
    last_sync: Option<Instant>,
    next_poll: Instant,
    command_running: bool,
    initial_refresh_pending: bool,
    exit_requested: bool,
    window_visible: bool,
    animations_enabled: bool,
    events: Receiver<UiEvent>,
    sender: Sender<UiEvent>,
    tray: Option<TrayIcon>,
    tray_status: Option<MenuItem>,
    tray_actions: HashMap<MenuId, TrayAction>,
}

impl App {
    fn new(
        creation: &eframe::CreationContext<'_>,
        sender: Sender<UiEvent>,
        events: Receiver<UiEvent>,
    ) -> Self {
        apply_theme(&creation.egui_ctx);
        let repaint = creation.egui_ctx.clone();
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = menu_sender.send(UiEvent::Menu(event));
            repaint.request_repaint();
        }));

        let mut startup_error = None;
        let preferences = load_preferences().unwrap_or_else(|error| {
            startup_error = Some(error);
            Preferences::default()
        });
        let mut credentials = match load_cloud_credentials() {
            Ok(credentials) => credentials,
            Err(error) => {
                startup_error.get_or_insert(error);
                None
            }
        };

        if credentials.is_none()
            && let Some((legacy_path, legacy_settings)) = read_legacy_settings()
        {
            match parse_cloud_oauth(&legacy_settings) {
                Ok(Some(oauth)) => match save_cloud_credentials(&oauth) {
                    Ok(()) => {
                        credentials = Some(oauth);
                        if let Err(error) = redact_legacy_credentials(&legacy_path, legacy_settings)
                        {
                            startup_error.get_or_insert(error);
                        }
                    }
                    Err(error) => {
                        startup_error.get_or_insert(error);
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    startup_error.get_or_insert(error);
                }
            }
        }

        if !app_settings_path().is_ok_and(|path| path.is_file())
            && let Err(error) = save_preferences(&preferences)
        {
            startup_error.get_or_insert(error);
        }

        let connections = match build_connections(&preferences, credentials.clone()) {
            Ok(connections) => connections,
            Err(error) => {
                startup_error.get_or_insert(error);
                Connections::default()
            }
        };
        let export_devices = load_export_devices(&preferences.local_export).unwrap_or_default();
        let cloud_minimum = preferences
            .cloud_white_range
            .map(|range| range.0.to_string())
            .unwrap_or_default();
        let cloud_maximum = preferences
            .cloud_white_range
            .map(|range| range.1.to_string())
            .unwrap_or_default();
        let mut app = Self {
            page: if connections.has(preferences.backend) {
                Page::Home
            } else {
                Page::Connections
            },
            setup_tab: preferences.backend,
            preferences: preferences.clone(),
            draft: preferences,
            connections,
            export_devices,
            cloud_credentials: credentials,
            google_client_id: String::new(),
            google_client_secret: String::new(),
            wiz_client_id: String::new(),
            wiz_client_secret: String::new(),
            replace_credentials: false,
            cloud_minimum,
            cloud_maximum,
            pilot: None,
            temperature: 4000,
            timer_minutes: 30,
            timer_due: None,
            status: startup_error.unwrap_or_else(|| "Preparando WiZ…".to_string()),
            last_error: None,
            last_sync: None,
            next_poll: Instant::now(),
            command_running: false,
            initial_refresh_pending: true,
            exit_requested: false,
            window_visible: true,
            animations_enabled: client_area_animations_enabled(),
            events,
            sender,
            tray: None,
            tray_status: None,
            tray_actions: HashMap::new(),
        };
        if let Err(error) = app.install_tray() {
            app.set_status(&format!("Dashboard listo; la bandeja no inició: {error}"));
        }
        app
    }

    fn install_tray(&mut self) -> Result<(), String> {
        let menu = Menu::new();
        let status = MenuItem::new(&format!("WiZ · {}", self.draft.device_name), false, None);
        menu.append(&status).map_err(|error| error.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        append_tray_action(
            &menu,
            "Abrir dashboard",
            TrayAction::Open,
            &mut self.tray_actions,
        )?;
        append_tray_action(
            &menu,
            "Luz de día",
            TrayAction::Control(Action::Daylight),
            &mut self.tray_actions,
        )?;
        append_tray_action(
            &menu,
            "Encender",
            TrayAction::Control(Action::On),
            &mut self.tray_actions,
        )?;
        append_tray_action(
            &menu,
            "Apagar",
            TrayAction::Control(Action::Off),
            &mut self.tray_actions,
        )?;
        for (scene_id, name) in SCENES.iter().skip(1) {
            append_tray_action(
                &menu,
                &format!("Modo · {name}"),
                TrayAction::Control(Action::Scene(*scene_id)),
                &mut self.tray_actions,
            )?;
        }
        for temperature in [2700, 3500, 4000, 5000, 6500] {
            append_tray_action(
                &menu,
                &format!("Temperatura · {temperature} K"),
                TrayAction::Control(Action::Temperature(temperature)),
                &mut self.tray_actions,
            )?;
        }
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        append_tray_action(
            &menu,
            "Conectar WiZ Cloud con Google",
            TrayAction::ConnectCloud,
            &mut self.tray_actions,
        )?;
        append_tray_action(
            &menu,
            "Actualizar estado",
            TrayAction::Control(Action::Refresh),
            &mut self.tray_actions,
        )?;
        append_tray_action(
            &menu,
            "Salir de WiZ Tray",
            TrayAction::Exit,
            &mut self.tray_actions,
        )?;

        self.tray = Some(
            TrayIconBuilder::new()
                .with_icon(make_icon(false)?)
                .with_tooltip("WiZ · preparando dashboard")
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(true)
                .build()
                .map_err(|error| error.to_string())?,
        );
        self.tray_status = Some(status);
        Ok(())
    }

    fn set_status(&mut self, text: &str) {
        self.status = text.to_string();
        if let Some(status) = &self.tray_status {
            let _ = status.set_text(text);
        }
        if let Some(tray) = &self.tray {
            let _ = tray.set_tooltip(Some(text));
        }
    }

    fn start_action(&mut self, backend: Backend, action: Action, ctx: &egui::Context) {
        if self.command_running {
            return;
        }
        if !self.connections.has(backend) {
            self.setup_tab = backend;
            self.page = Page::Connections;
            self.set_status("Configura esta conexión para continuar.");
            ctx.request_repaint();
            return;
        }
        if backend == Backend::Cloud && matches!(action, Action::Connect) {
            self.set_status("Abriendo el inicio de sesión de Google…");
        } else if matches!(action, Action::Refresh) {
            self.set_status("Consultando el estado del foco…");
        } else {
            self.set_status("Enviando comando y verificando el estado…");
        }
        self.command_running = true;
        self.last_error = None;
        let connections = self.connections.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = connections.perform(backend, &action);
            let _ = sender.send(UiEvent::Finished(backend, result));
        });
        ctx.request_repaint();
    }

    fn finish_action(&mut self, backend: Backend, result: Result<Pilot, String>) {
        self.command_running = false;
        self.last_sync = Some(Instant::now());
        self.next_poll = Instant::now() + poll_interval(backend);
        match result {
            Ok(pilot) => {
                self.pilot = Some(pilot);
                let message = pilot_status(pilot);
                self.last_error = None;
                self.set_status(&format!("{} · {message}", self.preferences.device_name));
                if let Some(temperature) = pilot.temp {
                    self.temperature = temperature;
                }
                if let Some(tray) = &self.tray
                    && let Ok(icon) = make_icon(pilot.state)
                {
                    let _ = tray.set_icon(Some(icon));
                }
            }
            Err(error) => {
                self.last_error = Some(error.clone());
                self.set_status(&error);
            }
        }
    }

    fn set_backend(&mut self, backend: Backend) {
        if !self.connections.has(backend) || self.preferences.backend == backend {
            return;
        }
        self.preferences.backend = backend;
        self.draft.backend = backend;
        self.pilot = None;
        self.last_sync = None;
        self.next_poll = Instant::now();
        if let Err(error) = save_preferences(&self.preferences) {
            self.last_error = Some(error.clone());
            self.set_status(&error);
        } else {
            self.set_status(&format!("Conexión activa · {}", backend.label()));
        }
    }

    fn use_cloud_export_device(&mut self, device: &ExportDevice) {
        if self.draft.device_name == "Mi foco" {
            self.draft.device_name = device.name.clone();
        }
        self.draft.cloud_device_id = device
            .device_id
            .map(|id| id.to_string())
            .unwrap_or_default();
        self.draft.cloud_home_id = device.home_id.map(|id| id.to_string()).unwrap_or_default();
        self.draft.cloud_mac = device.mac.clone();
        self.draft.cloud_white_range = device.white_range;
        self.cloud_minimum = device
            .white_range
            .map(|range| range.0.to_string())
            .unwrap_or_default();
        self.cloud_maximum = device
            .white_range
            .map(|range| range.1.to_string())
            .unwrap_or_default();
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                UiEvent::Finished(backend, result) => self.finish_action(backend, result),
                UiEvent::Menu(event) => {
                    let Some(action) = self.tray_actions.get(&event.id).cloned() else {
                        continue;
                    };
                    match action {
                        TrayAction::Open => {
                            self.window_visible = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                            self.page = if self.connections.has(self.preferences.backend) {
                                Page::Home
                            } else {
                                Page::Connections
                            };
                        }
                        TrayAction::Control(action) => {
                            self.start_action(self.preferences.backend, action, ctx);
                        }
                        TrayAction::ConnectCloud => {
                            self.set_backend(Backend::Cloud);
                            self.page = Page::Home;
                            self.start_action(Backend::Cloud, Action::Connect, ctx);
                        }
                        TrayAction::Exit => {
                            self.exit_requested = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            }
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        self.handle_events(ctx);
        if self.initial_refresh_pending {
            self.initial_refresh_pending = false;
            if self.connections.has(self.preferences.backend)
                && (self.preferences.backend != Backend::Cloud || has_cloud_session())
            {
                self.start_action(self.preferences.backend, Action::Refresh, ctx);
            } else if self.preferences.backend == Backend::Cloud
                && self.connections.has(Backend::Cloud)
            {
                self.set_status("WiZ Cloud listo · inicia sesión para leer el foco.");
            }
        }
        if self.timer_due.is_some_and(|due| Instant::now() >= due) && !self.command_running {
            self.timer_due = None;
            self.start_action(self.preferences.backend, Action::Off, ctx);
        }
        if !self.command_running
            && self.connections.has(self.preferences.backend)
            && (self.preferences.backend != Backend::Cloud || has_cloud_session())
            && Instant::now() >= self.next_poll
        {
            self.start_action(self.preferences.backend, Action::Refresh, ctx);
        }
        let delay = if let Some(due) = self.timer_due {
            if self.window_visible {
                Duration::from_secs(1)
            } else {
                due.saturating_duration_since(Instant::now())
                    .max(Duration::from_secs(1))
            }
        } else if self.window_visible
            && self.animations_enabled
            && self.pilot.is_some_and(|pilot| pilot.state)
        {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(5)
        };
        ctx.request_repaint_after(delay);
    }

    fn draw(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let area = ui.available_size();
        let sidebar_width = 196.0_f32.min((area.x * 0.28).max(166.0));
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, area.y),
                Layout::top_down(Align::Min),
                |ui| self.draw_sidebar(ui),
            );
            ui.separator();
            ui.add_space(8.0);
            let main_width = ui.available_width().max(1.0);
            ui.allocate_ui_with_layout(
                Vec2::new(main_width, area.y),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::Frame::new()
                        .fill(rgb(17, 21, 19))
                        .stroke(Stroke::NONE)
                        .inner_margin(Margin::symmetric(16, 14))
                        .show(ui, |ui| {
                            self.draw_header(ui);
                            ui.add_space(12.0);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| match self.page {
                                    Page::Home => self.draw_home(ui, ctx),
                                    Page::Routines => self.draw_routines(ui, ctx),
                                    Page::Connections => self.draw_connections(ui, ctx),
                                });
                        });
                },
            );
        });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(rgb(14, 18, 16))
            .inner_margin(egui::Margin::symmetric(20, 24))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("WiZ")
                        .size(27.0)
                        .strong()
                        .color(rgb(224, 175, 99)),
                );
                ui.label(
                    RichText::new("LUZ, A TU RITMO")
                        .size(10.0)
                        .color(rgb(137, 148, 140)),
                );
                ui.add_space(42.0);
                ui.label(
                    RichText::new("TU ESPACIO")
                        .size(9.0)
                        .strong()
                        .color(rgb(104, 116, 107)),
                );
                ui.add_space(10.0);
                self.nav_button(ui, Page::Home, "Control");
                self.nav_button(ui, Page::Routines, "Rutinas");
                self.nav_button(ui, Page::Connections, "Conexiones");
                ui.add_space(30.0);
                ui.separator();
                ui.add_space(16.0);
                ui.label(
                    RichText::new("FOCO ACTUAL")
                        .size(9.0)
                        .strong()
                        .color(rgb(104, 116, 107)),
                );
                ui.add_space(9.0);
                let color = if self.pilot.is_some_and(|pilot| pilot.state) {
                    rgb(224, 175, 99)
                } else {
                    rgb(115, 127, 119)
                };
                egui::Frame::new()
                    .fill(rgb(24, 30, 27))
                    .stroke(Stroke::new(1.0, rgb(39, 47, 42)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("●").color(color).size(12.0));
                            ui.label(
                                RichText::new(&self.preferences.device_name)
                                    .size(12.0)
                                    .strong()
                                    .color(rgb(231, 233, 226)),
                            );
                        });
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(self.preferences.backend.label())
                                .size(10.0)
                                .color(rgb(139, 150, 142)),
                        );
                    });
            });
    }

    fn nav_button(&mut self, ui: &mut egui::Ui, page: Page, label: &str) {
        let selected = self.page == page;
        let color = if selected {
            rgb(48, 43, 32)
        } else {
            rgb(14, 18, 16)
        };
        let label_text = RichText::new(label).size(13.0).color(if selected {
            rgb(229, 184, 108)
        } else {
            rgb(181, 188, 180)
        });
        let label_text = if selected {
            label_text.strong()
        } else {
            label_text
        };
        let response = ui.add_sized(
            [ui.available_width(), 38.0],
            egui::Button::new(label_text)
                .fill(color)
                .corner_radius(CornerRadius::same(8))
                .stroke(Stroke::NONE),
        );
        if response.clicked() {
            self.page = page;
        }
        ui.add_space(4.0);
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let heading = match self.page {
                Page::Home => "Panel de luz",
                Page::Routines => "Rutinas",
                Page::Connections => "Configurar WiZ",
            };
            ui.label(
                RichText::new(heading)
                    .size(25.0)
                    .strong()
                    .color(rgb(238, 238, 230)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.connections.has(Backend::Cloud) {
                    let mut connect = false;
                    if self.preferences.backend == Backend::Cloud && !has_cloud_session() {
                        connect = ui.button("Iniciar sesión").clicked();
                    }
                    if connect {
                        self.start_action(Backend::Cloud, Action::Connect, ui.ctx());
                    }
                }
                let mut choose_local = false;
                let mut choose_cloud = false;
                if self.connections.has(Backend::Cloud) {
                    choose_cloud = ui
                        .selectable_label(self.preferences.backend == Backend::Cloud, "Cloud")
                        .clicked();
                }
                if self.connections.has(Backend::Local) {
                    choose_local = ui
                        .selectable_label(self.preferences.backend == Backend::Local, "Casa")
                        .clicked();
                }
                if choose_local {
                    self.set_backend(Backend::Local);
                } else if choose_cloud {
                    self.set_backend(Backend::Cloud);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&self.status)
                    .size(11.0)
                    .color(rgb(150, 159, 151)),
            );
            if let Some(sync) = self.last_sync {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("Actualizado hace {} s", sync.elapsed().as_secs()))
                            .size(10.0)
                            .color(rgb(119, 132, 123)),
                    );
                });
            }
        });
    }

    fn draw_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.connections.has(self.preferences.backend) {
            self.draw_setup_prompt(ui);
            return;
        }
        self.draw_hero_card(ui, ctx);
        ui.add_space(16.0);
        let available = ui.available_width();
        if available >= 720.0 {
            ui.columns(2, |columns| {
                self.draw_temperature_card(&mut columns[0], ctx);
                self.draw_scenes_card(&mut columns[1], ctx);
            });
        } else {
            self.draw_temperature_card(ui, ctx);
            ui.add_space(14.0);
            self.draw_scenes_card(ui, ctx);
        }
    }

    fn draw_setup_prompt(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Conecta tu foco").size(24.0).strong());
            ui.add_space(8.0);
            ui.label("Configura una conexión local para usar la red de casa, o WiZ Cloud para controlar el foco desde cualquier lugar.");
            ui.add_space(18.0);
            if ui.button("Abrir configuración").clicked() {
                self.page = Page::Connections;
            }
        });
    }

    fn draw_hero_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let state = self.pilot.map(|pilot| pilot.state);
        let mode = self
            .pilot
            .map(|pilot| scene_name(pilot.scene_id).to_string())
            .unwrap_or_else(|| "Esperando estado del foco".to_string());
        egui::Frame::new()
            .fill(rgb(24, 30, 27))
            .stroke(Stroke::new(1.0, rgb(43, 52, 46)))
            .corner_radius(CornerRadius::same(18))
            .inner_margin(Margin::same(24))
            .show(ui, |ui| {
                let content_width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(content_width * 0.56, 276.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.label(
                                RichText::new(if state == Some(true) {
                                    "●  LUZ ACTIVA"
                                } else if state == Some(false) {
                                    "●  LUZ EN PAUSA"
                                } else {
                                    "●  SIN LECTURA"
                                })
                                .size(10.0)
                                .strong()
                                .color(if state == Some(true) {
                                    rgb(230, 184, 106)
                                } else {
                                    rgb(142, 153, 145)
                                }),
                            );
                            ui.add_space(23.0);
                            ui.label(
                                RichText::new(&self.preferences.device_name)
                                    .size(28.0)
                                    .strong()
                                    .color(rgb(240, 239, 231)),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(match state {
                                    Some(true) => "Encendida",
                                    Some(false) => "Apagada",
                                    None => "Sin lectura",
                                })
                                .size(39.0)
                                .strong()
                                .color(if state == Some(true) {
                                    rgb(235, 187, 107)
                                } else {
                                    rgb(220, 223, 216)
                                }),
                            );
                            ui.label(RichText::new(mode).size(13.0).color(rgb(161, 170, 162)));
                            ui.add_space(22.0);
                            ui.horizontal(|ui| {
                                let toggle_label = if state == Some(true) {
                                    "Apagar"
                                } else {
                                    "Encender"
                                };
                                if ui
                                    .add_sized(
                                        [122.0, 44.0],
                                        egui::Button::new(
                                            RichText::new(toggle_label)
                                                .strong()
                                                .color(rgb(28, 29, 25)),
                                        )
                                        .fill(rgb(226, 177, 98))
                                        .corner_radius(CornerRadius::same(9))
                                        .stroke(Stroke::NONE),
                                    )
                                    .clicked()
                                {
                                    let action = if state == Some(true) {
                                        Action::Off
                                    } else {
                                        Action::Daylight
                                    };
                                    self.start_action(self.preferences.backend, action, ctx);
                                }
                                if ui
                                    .add_sized(
                                        [130.0, 44.0],
                                        egui::Button::new(
                                            RichText::new("Luz de día").color(rgb(226, 190, 128)),
                                        )
                                        .fill(rgb(37, 45, 40))
                                        .corner_radius(CornerRadius::same(9))
                                        .stroke(Stroke::new(1.0, rgb(57, 66, 60))),
                                    )
                                    .clicked()
                                {
                                    self.start_action(
                                        self.preferences.backend,
                                        Action::Daylight,
                                        ctx,
                                    );
                                }
                            });
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new(format!(
                                    "Conexión · {}",
                                    self.preferences.backend.label()
                                ))
                                .size(11.0)
                                .color(rgb(133, 145, 136)),
                            );
                        },
                    );
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        Vec2::new(content_width * 0.39, 276.0),
                        Layout::top_down(Align::Center),
                        |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 276.0),
                                Sense::hover(),
                            );
                            paint_bulb(
                                ui.painter(),
                                rect,
                                self.pilot,
                                self.animations_enabled,
                                ctx.input(|input| input.time),
                            );
                        },
                    );
                });
            });
    }

    fn draw_temperature_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((minimum, maximum)) = self.connections.white_range(self.preferences.backend)
        else {
            card(ui, |ui| {
                ui.label(RichText::new("Temperatura de color").size(17.0).strong());
                ui.label("Este foco no informa un rango de blanco variable en su exportación.");
            });
            return;
        };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Temperatura").size(17.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} K", self.temperature))
                            .size(18.0)
                            .strong()
                            .color(rgb(230, 184, 106)),
                    );
                });
            });
            self.temperature = self.temperature.clamp(minimum, maximum);
            ui.spacing_mut().slider_width = ui.available_width();
            let response = ui.add(
                egui::Slider::new(&mut self.temperature, minimum..=maximum)
                    .step_by(100.0)
                    .show_value(false),
            );
            if response.changed() {
                ctx.request_repaint();
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{minimum} K · cálida"))
                        .size(10.0)
                        .color(rgb(196, 161, 112)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{maximum} K · fría"))
                            .size(10.0)
                            .color(rgb(146, 176, 194)),
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for temperature in temperature_options((minimum, maximum)) {
                    if ui
                        .selectable_label(
                            self.temperature == temperature,
                            format!("{temperature} K"),
                        )
                        .clicked()
                    {
                        self.temperature = temperature;
                        self.start_action(
                            self.preferences.backend,
                            Action::Temperature(temperature),
                            ctx,
                        );
                    }
                }
            });
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Aplicar temperatura")
                            .strong()
                            .color(rgb(28, 29, 25)),
                    )
                    .fill(rgb(226, 177, 98))
                    .corner_radius(CornerRadius::same(8))
                    .stroke(Stroke::NONE),
                )
                .clicked()
            {
                self.start_action(
                    self.preferences.backend,
                    Action::Temperature(self.temperature),
                    ctx,
                );
            }
        });
    }

    fn draw_scenes_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            ui.label(RichText::new("Escenas").size(17.0).strong());
            ui.add_space(10.0);
            let scene_count = SCENES.len();
            for row in 0..scene_count.div_ceil(2) {
                ui.horizontal(|ui| {
                    for column in 0..2 {
                        let index = row * 2 + column;
                        if let Some((scene_id, name)) = SCENES.get(index) {
                            let active =
                                self.pilot.and_then(|pilot| pilot.scene_id) == Some(*scene_id);
                            if ui
                                .add_sized(
                                    [ui.available_width() / (2 - column) as f32 - 7.0, 48.0],
                                    egui::Button::new(
                                        RichText::new(if active {
                                            format!("●  {name}")
                                        } else {
                                            (*name).to_string()
                                        })
                                        .color(
                                            if active {
                                                rgb(230, 184, 106)
                                            } else {
                                                rgb(203, 209, 201)
                                            },
                                        ),
                                    )
                                    .fill(if active {
                                        rgb(54, 46, 32)
                                    } else {
                                        rgb(30, 37, 33)
                                    })
                                    .corner_radius(CornerRadius::same(9))
                                    .stroke(Stroke::new(
                                        1.0,
                                        if active {
                                            rgb(107, 82, 43)
                                        } else {
                                            rgb(45, 54, 48)
                                        },
                                    )),
                                )
                                .clicked()
                            {
                                self.start_action(
                                    self.preferences.backend,
                                    Action::Scene(*scene_id),
                                    ctx,
                                );
                            }
                        }
                    }
                });
            }
        });
    }

    fn draw_routines(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            ui.label(RichText::new("Apagado automático").size(20.0).strong());
            ui.label("Programa el apagado del foco después de un intervalo.");
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Slider::new(&mut self.timer_minutes, 5..=180)
                        .step_by(5.0)
                        .suffix(" min"),
                );
                if self.timer_due.is_none() {
                    if ui.button("Iniciar rutina").clicked() {
                        self.timer_due = Some(
                            Instant::now()
                                + Duration::from_secs(u64::from(self.timer_minutes) * 60),
                        );
                        self.set_status(
                            format!("Apagado programado en {} minutos.", self.timer_minutes)
                                .as_str(),
                        );
                    }
                } else if ui.button("Cancelar").clicked() {
                    self.timer_due = None;
                    self.set_status("Rutina cancelada.");
                }
            });
            if let Some(due) = self.timer_due {
                let remaining = due.saturating_duration_since(Instant::now()).as_secs();
                ui.label(
                    RichText::new(format!(
                        "Se apagará en {:02}:{:02}.",
                        remaining / 60,
                        remaining % 60
                    ))
                    .size(14.0)
                    .strong()
                    .color(rgb(230, 184, 106)),
                );
            }
            ui.add_space(12.0);
            ui.label(RichText::new("La rutina sigue activa al ocultar la ventana en la bandeja y se cancela al salir de WiZ Tray.").size(11.0).color(rgb(155, 165, 156)));
        });
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.label(RichText::new("Accesos rápidos").size(16.0).strong());
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                for (label, action) in [
                    ("Día de trabajo", Action::Scene(15)),
                    ("Relajarse", Action::Scene(16)),
                    ("Luz nocturna", Action::Scene(14)),
                    ("Luz de día", Action::Daylight),
                ] {
                    if ui
                        .add_sized([150.0, 42.0], egui::Button::new(label))
                        .clicked()
                    {
                        self.start_action(self.preferences.backend, action, ctx);
                    }
                }
            });
        });
    }

    fn draw_connections(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.setup_tab == Backend::Local, "Local · misma red")
                .clicked()
            {
                self.setup_tab = Backend::Local;
            }
            if ui
                .selectable_label(self.setup_tab == Backend::Cloud, "WiZ Cloud · remoto")
                .clicked()
            {
                self.setup_tab = Backend::Cloud;
            }
        });
        ui.add_space(12.0);
        match self.setup_tab {
            Backend::Local => self.draw_local_setup(ui, ctx),
            Backend::Cloud => self.draw_cloud_setup(ui, ctx),
        }
    }

    fn draw_local_setup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            ui.label(RichText::new("Conexión local").size(20.0).strong());
            ui.label("Controla el foco directamente por Wi-Fi. No necesita iniciar sesión ni depender de WiZ Cloud.");
            ui.add_space(16.0);
            ui.label(RichText::new("1 · Activa el control local").strong());
            ui.label("En WiZ: Ajustes → Seguridad → activa la comunicación local.");
            ui.add_space(8.0);
            ui.label(RichText::new("2 · Importa el export de tu hogar").strong());
            ui.horizontal(|ui| {
                let displayed = if self.draft.local_export.is_empty() {
                    "Aún no has elegido wiz-home.json".to_string()
                } else {
                    Path::new(&self.draft.local_export)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Export WiZ seleccionado")
                        .to_string()
                };
                ui.label(
                    RichText::new(displayed)
                        .size(11.0)
                        .color(rgb(155, 165, 156)),
                );
                if ui.button("Elegir archivo…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Export WiZ", &["json"])
                        .set_title("Selecciona el export local de WiZ")
                        .pick_file()
                {
                    match load_export_devices(path.to_string_lossy().as_ref()) {
                        Ok(devices) => {
                            self.draft.local_export = path.to_string_lossy().into_owned();
                            self.export_devices = devices;
                            if let Some(device) = self.export_devices.first().cloned() {
                                self.draft.local_mac = device.mac.clone();
                                self.use_cloud_export_device(&device);
                            }
                            self.last_error = None;
                        }
                        Err(error) => self.last_error = Some(error),
                    }
                }
            });
            ui.label("El export contiene una clave privada de control. Se queda en tu equipo; no lo compartas ni lo subas a GitHub.");
            if !self.export_devices.is_empty() {
                egui::ComboBox::from_label("Foco del export")
                    .selected_text(selected_device_name(
                        &self.export_devices,
                        &self.draft.local_mac,
                    ))
                    .show_ui(ui, |ui| {
                        for device in &self.export_devices {
                            ui.selectable_value(
                                &mut self.draft.local_mac,
                                device.mac.clone(),
                                &device.name,
                            );
                        }
                    });
            }
            ui.add_space(8.0);
            ui.label(RichText::new("3 · Indica la dirección del foco").strong());
            ui.horizontal(|ui| {
                ui.label("IP local");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.local_ip)
                        .hint_text("192.168.1.25")
                        .desired_width(180.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Nombre");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.device_name)
                        .hint_text("Sala")
                        .desired_width(220.0),
                );
            });
            ui.add_space(12.0);
            if let Some(error) = &self.last_error {
                ui.colored_label(rgb(239, 140, 126), error);
            }
            if ui
                .add_sized(
                    [170.0, 42.0],
                    egui::Button::new(
                        RichText::new("Guardar y probar")
                            .strong()
                            .color(rgb(28, 29, 25)),
                    )
                    .fill(rgb(226, 177, 98))
                    .corner_radius(CornerRadius::same(8))
                    .stroke(Stroke::NONE),
                )
                .clicked()
            {
                match self.save_local_setup() {
                    Ok(()) => self.start_action(Backend::Local, Action::Refresh, ctx),
                    Err(error) => self.last_error = Some(error),
                }
            }
            if self.connections.has(Backend::Local) {
                ui.label(
                    RichText::new("Configuración local guardada en este perfil de Windows.")
                        .size(11.0)
                        .color(rgb(144, 192, 148)),
                );
            }
        });
    }

    fn save_local_setup(&mut self) -> Result<(), String> {
        let local = build_local_config(&self.draft)?.ok_or_else(|| {
            "Elige el export, el foco y una dirección IP local válida.".to_string()
        })?;
        let selected = self
            .export_devices
            .iter()
            .find(|device| device.mac == self.draft.local_mac)
            .ok_or_else(|| "El foco seleccionado ya no aparece en el export WiZ.".to_string())?;
        if self.draft.device_name.trim().is_empty() {
            self.draft.device_name = selected.name.clone();
        }
        let backend_changed = self.preferences.backend != Backend::Local;
        self.draft.backend = Backend::Local;
        save_preferences(&self.draft)?;
        self.preferences = self.draft.clone();
        self.connections.local = Some(local);
        self.page = Page::Home;
        if backend_changed {
            self.pilot = None;
            self.last_sync = None;
        }
        self.next_poll = Instant::now();
        self.set_status("Conexión local guardada.");
        self.last_error = None;
        Ok(())
    }

    fn draw_cloud_setup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            ui.label(RichText::new("WiZ Cloud").size(20.0).strong());
            ui.label("Control remoto mientras este equipo siga encendido, conectado a internet y WiZ Tray permanezca abierto en la bandeja.");
            ui.add_space(10.0);
            ui.label("La API doméstica de WiZ no está documentada para terceros. Google confirma tu cuenta; WiZ requiere credenciales OAuth privadas propias, que no se incluyen en el repositorio.");
            ui.add_space(14.0);
            text_field(
                ui,
                "ID del foco",
                &mut self.draft.cloud_device_id,
                "12345678",
            );
            text_field(
                ui,
                "ID del hogar",
                &mut self.draft.cloud_home_id,
                "123456789",
            );
            text_field(
                ui,
                "MAC del foco",
                &mut self.draft.cloud_mac,
                "AA:BB:CC:DD:EE:FF",
            );
            ui.horizontal(|ui| {
                if ui.button("Importar export WiZ…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Export WiZ", &["json"])
                        .set_title("Selecciona el export local de WiZ")
                        .pick_file()
                {
                    match load_export_devices(path.to_string_lossy().as_ref()) {
                        Ok(devices) => {
                            self.export_devices = devices;
                            self.draft.local_export = path.to_string_lossy().into_owned();
                            if let Some(device) = self.export_devices.first().cloned() {
                                self.draft.local_mac = device.mac.clone();
                                self.use_cloud_export_device(&device);
                            }
                            self.last_error = None;
                        }
                        Err(error) => self.last_error = Some(error),
                    }
                }
                if !self.export_devices.is_empty() {
                    let devices = self.export_devices.clone();
                    let mut selected_mac = self.draft.cloud_mac.clone();
                    egui::ComboBox::from_label("Foco")
                        .selected_text(selected_device_name(&devices, &selected_mac))
                        .show_ui(ui, |ui| {
                            for device in &devices {
                                ui.selectable_value(
                                    &mut selected_mac,
                                    device.mac.clone(),
                                    &device.name,
                                );
                            }
                        });
                    if selected_mac != self.draft.cloud_mac
                        && let Some(device) =
                            devices.iter().find(|device| device.mac == selected_mac)
                    {
                        self.use_cloud_export_device(device);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Rango de blanco (K)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cloud_minimum)
                        .hint_text("2200")
                        .desired_width(80.0),
                );
                ui.label("a");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cloud_maximum)
                        .hint_text("6500")
                        .desired_width(80.0),
                );
                ui.label(
                    RichText::new("opcional")
                        .size(10.0)
                        .color(rgb(155, 165, 156)),
                );
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Credenciales de OAuth").strong());
                if self.cloud_credentials.is_some() && !self.replace_credentials {
                    ui.label(
                        RichText::new("Protegidas con Windows DPAPI")
                            .size(10.0)
                            .color(rgb(144, 192, 148)),
                    );
                    if ui.small_button("Reemplazar").clicked() {
                        self.replace_credentials = true;
                    }
                }
            });
            if self.cloud_credentials.is_none() || self.replace_credentials {
                text_field(
                    ui,
                    "Client ID de Google",
                    &mut self.google_client_id,
                    "...apps.googleusercontent.com",
                );
                secret_field(
                    ui,
                    "Secreto Google (si aplica)",
                    &mut self.google_client_secret,
                );
                secret_field(ui, "Client ID privado de WiZ", &mut self.wiz_client_id);
                secret_field(ui, "Secreto privado de WiZ", &mut self.wiz_client_secret);
                ui.label(RichText::new("Al guardar, estos valores se cifran con DPAPI y quedan ligados a tu perfil de Windows.").size(10.0).color(rgb(155, 165, 156)));
            }
            ui.add_space(12.0);
            if let Some(error) = &self.last_error {
                ui.colored_label(rgb(239, 140, 126), error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        [150.0, 42.0],
                        egui::Button::new(
                            RichText::new("Guardar Cloud")
                                .strong()
                                .color(rgb(28, 29, 25)),
                        )
                        .fill(rgb(226, 177, 98))
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::NONE),
                    )
                    .clicked()
                {
                    if let Err(error) = self.save_cloud_setup() {
                        self.last_error = Some(error);
                    }
                }
                if self.connections.has(Backend::Cloud)
                    && ui
                        .add_sized(
                            [180.0, 42.0],
                            egui::Button::new("Iniciar sesión con Google"),
                        )
                        .clicked()
                {
                    self.set_backend(Backend::Cloud);
                    self.page = Page::Home;
                    self.start_action(Backend::Cloud, Action::Connect, ctx);
                }
                if has_cloud_session() && ui.small_button("Desconectar sesión").clicked() {
                    if let Err(error) = clear_cloud_session() {
                        self.last_error = Some(error);
                    } else {
                        self.set_status("Sesión WiZ Cloud desconectada.");
                    }
                }
            });
            if self.connections.has(Backend::Cloud) {
                ui.label(
                    RichText::new(if has_cloud_session() {
                        "Sesión Cloud guardada en este perfil."
                    } else {
                        "Conexión configurada; falta iniciar sesión."
                    })
                    .size(11.0)
                    .color(rgb(144, 192, 148)),
                );
            }
        });
    }

    fn save_cloud_setup(&mut self) -> Result<(), String> {
        let cloud_white_range = parse_optional_range(&self.cloud_minimum, &self.cloud_maximum)?;
        let mut draft = self.draft.clone();
        draft.backend = Backend::Cloud;
        draft.cloud_white_range = cloud_white_range;
        let mut credentials_to_save = None;
        let mut oauth = self.cloud_credentials.clone();
        if self.replace_credentials
            || (self.cloud_credentials.is_none()
                && (!self.google_client_id.is_empty()
                    || !self.wiz_client_id.is_empty()
                    || !self.wiz_client_secret.is_empty()))
        {
            if self.google_client_id.trim().is_empty()
                || self.wiz_client_id.trim().is_empty()
                || self.wiz_client_secret.is_empty()
            {
                return Err("Completa los dos client ID y el secreto de WiZ. El secreto de Google es opcional.".to_string());
            }
            let credentials = CloudOAuthConfig {
                google_client_id: self.google_client_id.trim().to_string(),
                google_client_secret: (!self.google_client_secret.is_empty())
                    .then(|| self.google_client_secret.clone()),
                wiz_client_id: self.wiz_client_id.trim().to_string(),
                wiz_client_secret: self.wiz_client_secret.clone(),
            };
            credentials_to_save = Some(credentials.clone());
            oauth = Some(credentials);
        }
        let cloud = build_cloud_config(&draft, oauth.clone())?
            .ok_or_else(|| "Completa el ID del foco, el ID del hogar y la MAC.".to_string())?;
        if let Some(credentials) = &credentials_to_save {
            save_cloud_credentials(credentials)?;
        }
        save_preferences(&draft)?;
        let backend_changed = self.preferences.backend != Backend::Cloud;
        self.preferences = draft.clone();
        self.draft = draft;
        self.connections.cloud = Some(cloud);
        self.page = Page::Home;
        self.cloud_credentials = oauth;
        if backend_changed {
            self.pilot = None;
            self.last_sync = None;
        }
        self.next_poll = Instant::now();
        self.set_status("Conexión WiZ Cloud guardada.");
        if credentials_to_save.is_some() {
            self.google_client_id.clear();
            self.google_client_secret.clear();
            self.wiz_client_id.clear();
            self.wiz_client_secret.clear();
            self.replace_credentials = false;
        }
        self.last_error = None;
        Ok(())
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|input| input.viewport().close_requested()) && !self.exit_requested {
            self.window_visible = false;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }
        let ctx = ui.ctx().clone();
        egui::Frame::new()
            .fill(rgb(17, 21, 19))
            .show(ui, |ui| self.draw(ui, &ctx));
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = rgb(17, 21, 19);
    visuals.window_fill = rgb(21, 26, 23);
    visuals.extreme_bg_color = rgb(15, 19, 17);
    visuals.faint_bg_color = rgb(23, 29, 26);
    visuals.selection.bg_fill = rgb(69, 54, 31);
    visuals.selection.stroke = Stroke::new(1.0, rgb(226, 177, 98));
    visuals.widgets.noninteractive.bg_fill = rgb(25, 31, 28);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, rgb(229, 232, 224));
    visuals.widgets.inactive.bg_fill = rgb(30, 37, 33);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, rgb(203, 210, 201));
    visuals.widgets.hovered.bg_fill = rgb(43, 52, 46);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, rgb(240, 239, 231));
    visuals.widgets.active.bg_fill = rgb(226, 177, 98);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, rgb(28, 29, 25));
    visuals.widgets.open.bg_fill = rgb(43, 52, 46);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, rgb(240, 239, 231));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.open.corner_radius = CornerRadius::same(8);
    ctx.set_visuals(visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(12.0, 10.0);
        style.spacing.button_padding = Vec2::new(14.0, 9.0);
    });
}

fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(rgb(24, 30, 27))
        .stroke(Stroke::new(1.0, rgb(42, 51, 45)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Min), add_contents);
        });
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([170.0, 28.0], egui::Label::new(label));
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(270.0),
        );
    });
}

fn secret_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized([170.0, 28.0], egui::Label::new(label));
        ui.add(
            egui::TextEdit::singleline(value)
                .password(true)
                .desired_width(270.0),
        );
    });
}

fn paint_bulb(
    painter: &egui::Painter,
    rect: egui::Rect,
    pilot: Option<Pilot>,
    animate: bool,
    time: f64,
) {
    let scale = (rect.height() / 205.0).clamp(1.0, 1.42);
    let center = rect.center() + egui::vec2(0.0, -4.0 * scale);
    let on = pilot.is_some_and(|pilot| pilot.state);
    let pulse = if on && animate {
        (time as f32 * 0.7).sin() * 0.5 + 0.5
    } else {
        0.0
    };
    let warmth = pilot
        .and_then(|pilot| pilot.temp)
        .map(|temp| ((6500_u16.saturating_sub(temp).min(4300)) as f32 / 4300.0).clamp(0.0, 1.0))
        .unwrap_or(0.55);
    let glow = if warmth > 0.5 {
        rgb(245, 179, 94)
    } else {
        rgb(186, 211, 229)
    };
    if on && animate {
        for layer in (0..6).rev() {
            let radius = (43.0 + layer as f32 * 14.0 + pulse * 2.0) * scale;
            let alpha = 5 + (5 - layer) as u8 * 4;
            painter.circle_filled(
                center,
                radius,
                Color32::from_rgba_unmultiplied(glow.r(), glow.g(), glow.b(), alpha),
            );
        }
    }
    let fill = if on {
        rgb(255, 230, 174)
    } else {
        rgb(70, 80, 74)
    };
    let edge = if on {
        rgb(236, 190, 112)
    } else {
        rgb(132, 145, 136)
    };
    painter.circle_filled(center + egui::vec2(0.0, -14.0 * scale), 37.0 * scale, fill);
    painter.circle_stroke(
        center + egui::vec2(0.0, -14.0 * scale),
        37.0 * scale,
        Stroke::new(1.0, edge),
    );
    painter.rect_filled(
        egui::Rect::from_center_size(
            center + egui::vec2(0.0, 26.0 * scale),
            Vec2::new(22.0 * scale, 27.0 * scale),
        ),
        CornerRadius::same((5.0 * scale).round() as u8),
        fill,
    );
    for line in 0..3 {
        painter.line_segment(
            [
                center + egui::vec2(-12.0 * scale, (34.0 + line as f32 * 6.0) * scale),
                center + egui::vec2(12.0 * scale, (34.0 + line as f32 * 6.0) * scale),
            ],
            Stroke::new(2.0 * scale, edge),
        );
    }
    if on {
        for ray in 0..8 {
            let angle = std::f32::consts::TAU * ray as f32 / 8.0;
            let start = center
                + egui::vec2(
                    angle.cos() * 49.0 * scale,
                    angle.sin() * 49.0 * scale - 14.0 * scale,
                );
            let end = center
                + egui::vec2(
                    angle.cos() * 58.0 * scale,
                    angle.sin() * 58.0 * scale - 14.0 * scale,
                );
            painter.line_segment([start, end], Stroke::new(1.5 * scale, glow));
        }
    }
    painter.text(
        egui::pos2(center.x, rect.bottom() - 7.0),
        egui::Align2::CENTER_BOTTOM,
        if on { "EN LÍNEA" } else { "EN ESPERA" },
        egui::FontId::proportional(10.0),
        rgb(151, 161, 152),
    );
}

fn scene_name(scene_id: Option<u16>) -> &'static str {
    scene_id
        .and_then(|id| {
            SCENES
                .iter()
                .find(|(scene, _)| *scene == id)
                .map(|(_, name)| *name)
        })
        .unwrap_or("Modo actual")
}

fn pilot_status(pilot: Pilot) -> String {
    if !pilot.state {
        return "Apagada".to_string();
    }
    let scene = scene_name(pilot.scene_id);
    if scene != "Modo actual" {
        scene.to_string()
    } else {
        pilot
            .temp
            .map(|temperature| format!("{temperature} K"))
            .unwrap_or_else(|| "Encendida".to_string())
    }
}

fn selected_device_name(devices: &[ExportDevice], mac: &str) -> String {
    devices
        .iter()
        .find(|device| device.mac == mac)
        .map(|device| format!("{} · {}", device.name, display_mac(&device.mac)))
        .unwrap_or_else(|| "Selecciona un foco".to_string())
}

fn display_mac(mac: &str) -> String {
    mac.as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(":")
        .to_uppercase()
}

fn rgb(red: u8, green: u8, blue: u8) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

fn client_area_animations_enabled() -> bool {
    let mut enabled = 0;
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    result != 0 && enabled != 0
}

fn poll_interval(backend: Backend) -> Duration {
    match backend {
        Backend::Local => Duration::from_secs(20),
        Backend::Cloud => Duration::from_secs(60),
    }
}

fn append_tray_action(
    menu: &Menu,
    label: &str,
    action: TrayAction,
    actions: &mut HashMap<MenuId, TrayAction>,
) -> Result<(), String> {
    let item = MenuItem::new(label, true, None);
    menu.append(&item).map_err(|error| error.to_string())?;
    actions.insert(item.id().clone(), action);
    Ok(())
}

fn make_icon(on: bool) -> Result<Icon, String> {
    Icon::from_rgba(make_icon_rgba(on), 32, 32).map_err(|error| error.to_string())
}

fn make_icon_rgba(on: bool) -> Vec<u8> {
    let mut rgba = vec![0; 32 * 32 * 4];
    let bulb = if on {
        [248, 197, 105, 255]
    } else {
        [163, 176, 187, 255]
    };
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let background = dx * dx + dy * dy <= 225.0;
            let head = dx * dx / 56.0 + (y as f32 - 11.0).powi(2) / 76.0 <= 1.0;
            let neck = (11..=20).contains(&y) && (13..=18).contains(&x);
            let base = (21..=25).contains(&y) && (12..=19).contains(&x);
            let pixel = (y * 32 + x) * 4;
            if background {
                rgba[pixel..pixel + 4].copy_from_slice(&[39, 45, 48, 255]);
            }
            if head || neck || base {
                rgba[pixel..pixel + 4].copy_from_slice(&bulb);
            }
            if y == 23 && (13..=18).contains(&x) {
                rgba[pixel..pixel + 4].copy_from_slice(&[245, 239, 220, 255]);
            }
        }
    }
    rgba
}

fn preferences_json(preferences: &Preferences) -> Value {
    json!({
        "schema_version": 2,
        "backend": preferences.backend.as_str(),
        "device_name": preferences.device_name,
        "local": {
            "export_path": preferences.local_export,
            "device_ip": preferences.local_ip,
            "mac_address": preferences.local_mac,
        },
        "cloud": {
            "device_id": preferences.cloud_device_id,
            "home_id": preferences.cloud_home_id,
            "mac_address": preferences.cloud_mac,
            "white_range": preferences.cloud_white_range,
        }
    })
}

fn preferences_from_json(value: &Value) -> Result<Preferences, String> {
    if value.get("schema_version").and_then(Value::as_u64) != Some(2) {
        return Err("La configuración WiZ guardada usa una versión no compatible.".to_string());
    }
    let local = value.get("local").unwrap_or(&Value::Null);
    let cloud = value.get("cloud").unwrap_or(&Value::Null);
    let cloud_white_range = cloud
        .get("white_range")
        .filter(|value| !value.is_null())
        .map(|range| parse_white_range(&json!({"white_range": range})))
        .transpose()?;
    Ok(Preferences {
        backend: Backend::from_value(value.get("backend"))?,
        device_name: string_value(value.get("device_name"))
            .unwrap_or_else(|| "Mi foco".to_string()),
        local_export: string_value(local.get("export_path")).unwrap_or_default(),
        local_ip: string_value(local.get("device_ip")).unwrap_or_default(),
        local_mac: string_value(local.get("mac_address")).unwrap_or_default(),
        cloud_device_id: string_value(cloud.get("device_id")).unwrap_or_default(),
        cloud_home_id: string_value(cloud.get("home_id")).unwrap_or_default(),
        cloud_mac: string_value(cloud.get("mac_address")).unwrap_or_default(),
        cloud_white_range,
    })
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn app_settings_path() -> Result<PathBuf, String> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("WiZ Tray").join("settings.json"))
        .ok_or_else(|| "Windows no expuso la carpeta local para guardar WiZ Tray.".to_string())
}

fn legacy_settings_path() -> Option<PathBuf> {
    env::current_exe()
        .ok()?
        .parent()
        .map(|path| path.join("wiz-tray.json"))
}

fn load_preferences() -> Result<Preferences, String> {
    let path = app_settings_path()?;
    if path.is_file() {
        let bytes =
            fs::read(path).map_err(|_| "No pude leer la configuración de WiZ Tray.".to_string())?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "La configuración guardada de WiZ Tray no es JSON válido.".to_string())?;
        return preferences_from_json(&value);
    }
    if let Some((_, legacy)) = read_legacy_settings() {
        return preferences_from_legacy(&legacy);
    }
    Ok(Preferences::default())
}

fn read_legacy_settings() -> Option<(PathBuf, Value)> {
    let path = legacy_settings_path()?;
    let bytes = fs::read(&path).ok()?;
    let settings = serde_json::from_slice(&bytes).ok()?;
    Some((path, settings))
}

fn preferences_from_legacy(settings: &Value) -> Result<Preferences, String> {
    let backend = Backend::from_value(settings.get("backend"))?;
    let mut preferences = Preferences {
        backend,
        device_name: string_value(settings.get("device_name"))
            .unwrap_or_else(|| "Mi foco".to_string()),
        ..Preferences::default()
    };
    preferences.local_ip = string_value(settings.get("device_ip")).unwrap_or_default();
    if let Ok(path) = locate_export() {
        preferences.local_export = path.to_string_lossy().into_owned();
        if let Ok(devices) = load_export_devices(&preferences.local_export)
            && let Some(device) = devices.first()
        {
            preferences.local_mac = device.mac.clone();
            if preferences.device_name == "Mi foco" {
                preferences.device_name = device.name.clone();
            }
        }
    }
    if let (Ok(device), Ok(home), Some(mac)) = (
        parse_cloud_id(settings.get("device_id"), "device_id"),
        parse_cloud_id(settings.get("home_id"), "home_id"),
        settings.get("mac_address").and_then(Value::as_str),
    ) && normalize_mac(mac).len() == 12
    {
        preferences.cloud_device_id = device.to_string();
        preferences.cloud_home_id = home.to_string();
        preferences.cloud_mac = mac.to_string();
        preferences.cloud_white_range = settings
            .get("white_range")
            .map(|range| parse_white_range(&json!({"white_range": range})))
            .transpose()?;
    }
    Ok(preferences)
}

fn redact_legacy_credentials(path: &Path, mut settings: Value) -> Result<(), String> {
    if let Some(object) = settings.as_object_mut() {
        for field in [
            "google_oauth_client_id",
            "google_oauth_client_secret",
            "wiz_oauth_client_id",
            "wiz_oauth_client_secret",
        ] {
            object.remove(field);
        }
    }
    write_json(path, &settings)
}

fn save_preferences(preferences: &Preferences) -> Result<(), String> {
    let path = app_settings_path()?;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "No pude preparar la carpeta de configuración.".to_string())?,
    )
    .map_err(|_| "No pude preparar la carpeta de configuración de WiZ Tray.".to_string())?;
    write_json(&path, &preferences_json(preferences))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| "No pude preparar la configuración de WiZ Tray.".to_string())?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes)
        .map_err(|_| "No pude guardar la configuración de WiZ Tray.".to_string())?;
    replace_file(&temp, path)
        .map_err(|_| "No pude finalizar la configuración de WiZ Tray.".to_string())
}

fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn build_connections(
    preferences: &Preferences,
    oauth: Option<CloudOAuthConfig>,
) -> Result<Connections, String> {
    Ok(Connections {
        local: build_local_config(preferences)?,
        cloud: build_cloud_config(preferences, oauth)?,
    })
}

fn build_local_config(preferences: &Preferences) -> Result<Option<LightConfig>, String> {
    if preferences.local_export.trim().is_empty()
        || preferences.local_ip.trim().is_empty()
        || preferences.local_mac.trim().is_empty()
    {
        return Ok(None);
    }
    let ip = preferences
        .local_ip
        .trim()
        .parse::<Ipv4Addr>()
        .map_err(|_| "Escribe una dirección IPv4 local válida para el foco.".to_string())?;
    let bytes = fs::read(&preferences.local_export)
        .map_err(|_| "No pude abrir el export WiZ seleccionado.".to_string())?;
    let home: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "El export WiZ seleccionado no es JSON válido.".to_string())?;
    let mac = normalize_mac(&preferences.local_mac);
    let device = parse_export_devices(&home)?
        .into_iter()
        .find(|device| device.mac == mac)
        .ok_or_else(|| "El foco seleccionado no aparece en el export WiZ.".to_string())?;
    let key = find_signing_key(&home)
        .ok_or_else(|| "El export WiZ no contiene la clave de control local.".to_string())?;
    Ok(Some(LightConfig {
        ip,
        mac: device.mac,
        key: parse_key(key)?,
        white_range: device.white_range,
    }))
}

fn build_cloud_config(
    preferences: &Preferences,
    oauth: Option<CloudOAuthConfig>,
) -> Result<Option<CloudConfig>, String> {
    if preferences.cloud_device_id.trim().is_empty()
        && preferences.cloud_home_id.trim().is_empty()
        && preferences.cloud_mac.trim().is_empty()
    {
        return Ok(None);
    }
    let device_id = parse_cloud_id(
        Some(&Value::String(preferences.cloud_device_id.clone())),
        "device_id",
    )?;
    let home_id = parse_cloud_id(
        Some(&Value::String(preferences.cloud_home_id.clone())),
        "home_id",
    )?;
    let config = CloudConfig {
        device_id,
        home_id,
        mac_address: preferences.cloud_mac.trim().to_string(),
        white_range: preferences.cloud_white_range,
        oauth,
    };
    validate_cloud_config(&config)?;
    Ok(Some(config))
}

fn parse_optional_range(minimum: &str, maximum: &str) -> Result<Option<(u16, u16)>, String> {
    if minimum.trim().is_empty() && maximum.trim().is_empty() {
        return Ok(None);
    }
    let min = minimum
        .trim()
        .parse::<u16>()
        .map_err(|_| "El mínimo de temperatura debe ser un número en kelvin.".to_string())?;
    let max = maximum
        .trim()
        .parse::<u16>()
        .map_err(|_| "El máximo de temperatura debe ser un número en kelvin.".to_string())?;
    parse_white_range(&json!({"white_range": [min, max]})).map(Some)
}

fn load_export_devices(path: &str) -> Result<Vec<ExportDevice>, String> {
    let bytes =
        fs::read(path).map_err(|_| "No pude leer el export WiZ seleccionado.".to_string())?;
    let home: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "El export WiZ seleccionado no es JSON válido.".to_string())?;
    parse_export_devices(&home)
}

fn locate_export() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("WIZ_HOME_JSON") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err("La ruta WIZ_HOME_JSON no apunta a un archivo.".to_string());
    }
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .ok_or_else(|| "No pude localizar la carpeta de la aplicación.".to_string())?;
    let cwd = env::current_dir().unwrap_or_default();
    let candidates = [
        exe_dir.join("wiz-home.json"),
        exe_dir.join("..").join("wiz-home.json"),
        exe_dir.join("..").join("work").join("wiz-home.json"),
        cwd.join("wiz-home.json"),
        cwd.join("work").join("wiz-home.json"),
    ];
    candidates
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "No encuentro wiz-home.json. Impórtalo desde Conexiones para activar el control local."
                .to_string()
        })
}

pub fn show_error(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "WiZ Tray"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn run() -> Result<(), String> {
    let (sender, events) = mpsc::channel();
    let app_sender = sender.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WiZ Tray")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([860.0, 620.0])
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: make_icon_rgba(false),
                width: 32,
                height: 32,
            })),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "WiZ Tray",
        options,
        Box::new(move |creation| Ok(Box::new(App::new(creation, app_sender, events)))),
    )
    .map_err(|error| error.to_string())
}
