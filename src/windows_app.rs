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
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke,
    StrokeKind, Vec2,
};
use serde_json::{Value, json};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Foundation::SYSTEMTIME,
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::SystemInformation::GetLocalTime,
    UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MessageBoxW, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    },
};

use wiz_tray::{
    Action, CloudConfig, CloudOAuthConfig, DiscoveredLight, ExportDevice, LightConfig,
    PREFERRED_SCENE, Pilot, clear_cloud_session, discover_lights, find_signing_key,
    has_cloud_session, load_cloud_credentials, normalize_mac, parse_cloud_id, parse_cloud_oauth,
    parse_export_devices, parse_key, parse_white_range, perform, perform_cloud,
    save_cloud_credentials, temperature_options, validate_cloud_config,
};

const SCENES: &[(u16, &str)] = &[
    (PREFERRED_SCENE, "Luz de día"),
    (11, "Blanco cálido"),
    (13, "Blanco frío"),
    (15, "Enfoque"),
    (16, "Relajarse"),
    (14, "Luz nocturna"),
    (1, "Océano"),
    (2, "Romance"),
    (3, "Atardecer"),
    (4, "Fiesta"),
    (5, "Chimenea"),
    (6, "Acogedor"),
    (7, "Bosque"),
    (8, "Colores pastel"),
    (9, "Despertar"),
    (10, "Hora de dormir"),
    (17, "Colores reales"),
    (18, "Hora de TV"),
    (19, "Crecimiento de plantas"),
    (20, "Primavera"),
    (21, "Verano"),
    (22, "Otoño"),
    (23, "Inmersión"),
    (24, "Jungla"),
    (25, "Mojito"),
    (26, "Club"),
    (27, "Navidad"),
    (28, "Halloween"),
];

const FAVORITE_SCENES: &[u16] = &[PREFERRED_SCENE, 11, 13, 15, 16, 14];

const CANVAS: Color32 = Color32::from_rgb(13, 13, 15);
const SURFACE: Color32 = Color32::from_rgb(24, 24, 28);
const SURFACE_SOFT: Color32 = Color32::from_rgb(37, 37, 43);
const BORDER: Color32 = Color32::from_rgb(44, 44, 52);
const BORDER_STRONG: Color32 = Color32::from_rgb(74, 74, 86);
const INK: Color32 = Color32::from_rgb(246, 246, 249);
const MUTED: Color32 = Color32::from_rgb(148, 148, 160);
const ACCENT: Color32 = INK;
const ACCENT_DARK: Color32 = Color32::from_rgb(240, 178, 96);
const ACCENT_PALE: Color32 = SURFACE_SOFT;
const ON_ACCENT: Color32 = Color32::from_rgb(12, 12, 14);
const SUCCESS: Color32 = Color32::from_rgb(116, 214, 164);
const ERROR: Color32 = Color32::from_rgb(240, 138, 120);

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
            Self::Local => "Casa por Wi-Fi",
            Self::Cloud => "WiZ Cloud remoto",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoutineAction {
    Off,
    On,
    Daylight,
}

impl RoutineAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Daylight => "daylight",
        }
    }

    fn from_str(value: Option<&str>) -> Result<Self, String> {
        match value {
            Some("off") => Ok(Self::Off),
            Some("on") => Ok(Self::On),
            Some("daylight") => Ok(Self::Daylight),
            _ => Err("La acción guardada de la rutina diaria no es válida.".to_string()),
        }
    }

    fn action(self) -> Action {
        match self {
            Self::Off => Action::Off,
            Self::On => Action::On,
            Self::Daylight => Action::Daylight,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Apagar",
            Self::On => "Encender",
            Self::Daylight => "Encender con Luz de día",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DailyRoutine {
    enabled: bool,
    minute_of_day: u16,
    action: RoutineAction,
    last_run_day: Option<u32>,
}

impl Default for DailyRoutine {
    fn default() -> Self {
        Self {
            enabled: false,
            minute_of_day: 22 * 60,
            action: RoutineAction::Off,
            last_run_day: None,
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
    daily_routine: DailyRoutine,
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
            daily_routine: DailyRoutine::default(),
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
    Discovered(Result<Vec<DiscoveredLight>, String>),
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
    cloud_setup_step: usize,
    cloud_discovery_started: bool,
    pilot: Option<Pilot>,
    temperature: u16,
    brightness: u8,
    color: [u8; 3],
    discovered: Vec<DiscoveredLight>,
    discovery_running: bool,
    timer_minutes: u16,
    timer_due: Option<Instant>,
    routine_draft: DailyRoutine,
    show_all_scenes: bool,
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
        let routine_draft = preferences.daily_routine.clone();
        let mut app = Self {
            page: {
                if connections.has(preferences.backend) {
                    Page::Home
                } else {
                    Page::Connections
                }
            },
            setup_tab: preferences.backend,
            cloud_setup_step: 0,
            cloud_discovery_started: false,
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
            brightness: 100,
            color: [255, 214, 140],
            discovered: Vec::new(),
            discovery_running: false,
            timer_minutes: 30,
            timer_due: None,
            routine_draft,
            show_all_scenes: false,
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
        for (scene_id, name) in SCENES.iter().skip(1).take(FAVORITE_SCENES.len() - 1) {
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
                self.set_status(&format!("{}: {message}", self.preferences.device_name));
                if let Some(temperature) = pilot.temp {
                    self.temperature = temperature;
                }
                if let Some(dimming) = pilot.dimming {
                    self.brightness = dimming.clamp(1, 100);
                }
                if let Some((red, green, blue)) = pilot.color {
                    self.color = [red, green, blue];
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
            self.set_status(&format!("Conexión activa: {}", backend.label()));
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
                UiEvent::Discovered(result) => {
                    self.discovery_running = false;
                    match result {
                        Ok(lights) => {
                            self.discovered = lights;
                            let message = if self.discovered.is_empty() {
                                "No encontré focos en esta red. Revisa que el foco y la PC estén en la misma Wi-Fi."
                            } else {
                                "Focos encontrados. Elige uno para usarlo."
                            };
                            self.set_status(message);
                            self.last_error = None;
                        }
                        Err(error) => {
                            self.last_error = Some(error.clone());
                            self.set_status(&error);
                        }
                    }
                }
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

    fn start_discovery(&mut self, ctx: &egui::Context) {
        if self.discovery_running {
            return;
        }
        self.discovery_running = true;
        self.discovered.clear();
        self.last_error = None;
        self.set_status("Buscando focos WiZ en tu red local…");
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(UiEvent::Discovered(discover_lights()));
        });
        ctx.request_repaint();
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
                self.set_status("WiZ Cloud listo. Inicia sesión para leer el foco.");
            }
        }
        if self.timer_due.is_some_and(|due| Instant::now() >= due) && !self.command_running {
            self.timer_due = None;
            self.start_action(self.preferences.backend, Action::Off, ctx);
        }
        self.run_daily_routine(ctx);
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

    fn run_daily_routine(&mut self, ctx: &egui::Context) {
        let (today, minute) = local_day_and_minute();
        let Some(action) = routine_action_due(&self.preferences.daily_routine, today, minute)
        else {
            return;
        };

        if self.command_running {
            return;
        }
        let backend = self.preferences.backend;
        let skip_reason = if !self.connections.has(backend) {
            Some("Rutina diaria omitida: configura la conexión activa.")
        } else if backend == Backend::Cloud && !has_cloud_session() {
            Some("Rutina diaria omitida: inicia sesión en WiZ Cloud.")
        } else {
            None
        };

        self.preferences.daily_routine.last_run_day = Some(today);
        self.draft.daily_routine.last_run_day = Some(today);
        self.routine_draft.last_run_day = Some(today);
        if let Err(error) = save_preferences(&self.preferences) {
            self.last_error = Some(error.clone());
            self.set_status(&format!("No se ejecutó la rutina diaria: {error}"));
            return;
        }
        if let Some(reason) = skip_reason {
            self.set_status(reason);
            return;
        }
        self.start_action(backend, action, ctx);
    }

    fn draw(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let area = ui.available_size();
        let compact = area.x < 860.0;
        let accent = self.accent_color();
        egui::Frame::new()
            .inner_margin(Margin::symmetric(
                if compact { 16 } else { 26 },
                if compact { 14 } else { 18 },
            ))
            .show(ui, |ui| {
                self.draw_topbar(ui, compact);
                ui.add_space(12.0);
                self.draw_stage(ui, ctx, compact);
                ui.add_space(8.0);
                self.draw_tabs(ui, accent);
                ui.add_space(14.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Home => self.draw_home(ui, ctx),
                        Page::Routines => self.draw_routines(ui, ctx),
                        Page::Connections => self.draw_connections(ui, ctx),
                    });
            });
    }

    // El acento de la interfaz sigue al color real de la luz encendida.
    fn accent_color(&self) -> Color32 {
        match self.pilot.filter(|pilot| pilot.state) {
            Some(pilot) => mix(preview_light_color(Some(pilot), 0.0, false), INK, 0.18),
            None => INK,
        }
    }

    fn draw_stage(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
        let time = ui.input(|input| input.time) as f32;
        let on = self.pilot.is_some_and(|pilot| pilot.state);
        let light = preview_light_color(self.pilot, time, self.animations_enabled);
        let accent = if on { mix(light, INK, 0.18) } else { INK };
        let device = self.preferences.device_name.trim().to_owned();
        let device = if device.is_empty() {
            "Tu foco".to_owned()
        } else {
            device
        };
        let (state, subtitle) = match self.pilot {
            Some(pilot) if pilot.state => (
                scene_name(pilot.scene_id),
                "Encendida · el color muestra el estado que envía el foco",
            ),
            Some(_) => ("Apagada", "El foco está apagado"),
            None => ("Sin lectura", "Conecta un foco para ver su color en vivo"),
        };
        let radius = if compact { 34.0 } else { 46.0 };
        let halo = if compact { 16.0 } else { 26.0 };
        let pulse = if on && self.animations_enabled {
            0.9 + 0.1 * (time * 1.6).sin()
        } else {
            1.0
        };
        let brightness =
            f32::from(self.pilot.and_then(|pilot| pilot.dimming).unwrap_or(100)) / 100.0;
        let timer = self.timer_fraction();

        egui::Frame::new()
            .fill(SURFACE)
            .corner_radius(CornerRadius::same(22))
            .inner_margin(Margin::symmetric(
                if compact { 16 } else { 22 },
                if compact { 14 } else { 16 },
            ))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let size =
                        Vec2::new(radius * 2.0 + halo * 2.0, radius * 2.0 + halo * 2.0 + 14.0);
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    paint_bulb(
                        ui.painter(),
                        rect,
                        radius,
                        halo,
                        light,
                        on,
                        pulse,
                        brightness,
                        timer,
                    );
                    ui.add_space(if compact { 12.0 } else { 18.0 });
                    if compact {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(device).size(12.5).color(MUTED));
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                status_dot(ui, if on { accent } else { BORDER_STRONG });
                                ui.label(display(state, 21.0));
                            });
                            ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
                            ui.add_space(10.0);
                            self.draw_stage_metrics(ui, accent, false);
                            ui.add_space(10.0);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                self.draw_stage_actions(ui, ctx, accent, on);
                            });
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(device).size(12.5).color(MUTED));
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    status_dot(ui, if on { accent } else { BORDER_STRONG });
                                    ui.label(display(state, 26.0));
                                });
                                ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                ui.vertical(|ui| {
                                    self.draw_stage_metrics(ui, accent, true);
                                    ui.add_space(14.0);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        self.draw_stage_actions(ui, ctx, accent, on);
                                    });
                                });
                            });
                        });
                    }
                    if let Some(due) = self.timer_due {
                        ui.label(
                            RichText::new(format!(
                                "Se apaga en {} min",
                                due.saturating_duration_since(Instant::now())
                                    .as_secs()
                                    .div_ceil(60)
                            ))
                            .size(12.0)
                            .color(MUTED),
                        );
                    } else if self.preferences.daily_routine.enabled {
                        ui.label(
                            RichText::new("Rutina diaria programada")
                                .size(12.0)
                                .color(MUTED),
                        );
                    }
                });
            });
    }

    fn draw_stage_metrics(&mut self, ui: &mut egui::Ui, accent: Color32, stacked: bool) {
        let brightness = self
            .pilot
            .and_then(|pilot| pilot.dimming)
            .map(|value| format!("{value} %"))
            .unwrap_or_else(|| "—".to_string());
        let temperature = self
            .pilot
            .and_then(|pilot| pilot.temp)
            .map(|value| format!("{value} K"))
            .unwrap_or_else(|| "—".to_string());
        let signal = self
            .pilot
            .and_then(|pilot| pilot.rssi)
            .map(|value| format!("{value} dBm"))
            .unwrap_or_else(|| "—".to_string());
        let metrics = [
            ("Brillo", brightness.as_str()),
            ("Temperatura", temperature.as_str()),
            ("Señal", signal.as_str()),
        ];
        if stacked {
            let rssi = self.pilot.and_then(|pilot| pilot.rssi);
            ui.vertical(|ui| {
                for (label, value) in metrics {
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if label == "Señal" {
                                paint_signal(ui, rssi);
                            } else {
                                ui.add_space(22.0);
                            }
                            ui.label(
                                RichText::new(value)
                                    .size(15.0)
                                    .strong()
                                    .color(if value == "—" { MUTED } else { accent }),
                            );
                            ui.label(RichText::new(label).size(11.5).color(MUTED));
                        });
                    });
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in metrics {
                    hero_stat(ui, label, value, accent);
                    ui.add_space(22.0);
                }
            });
        }
    }

    fn draw_stage_actions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        accent: Color32,
        on: bool,
    ) {
        if ui
            .add_sized(
                [104.0, 34.0],
                egui::Button::new(
                    RichText::new(if on { "Apagar" } else { "Encender" })
                        .size(13.0)
                        .color(ON_ACCENT),
                )
                .fill(if on { accent } else { INK })
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(9)),
            )
            .clicked()
        {
            self.start_action(
                self.preferences.backend,
                if on { Action::Off } else { Action::Daylight },
                ctx,
            );
        }
        if ui
            .add_sized(
                [104.0, 34.0],
                egui::Button::new(RichText::new("Luz de día").size(13.0).color(INK))
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::new(1.0, BORDER_STRONG))
                    .corner_radius(CornerRadius::same(9)),
            )
            .clicked()
        {
            self.start_action(self.preferences.backend, Action::Daylight, ctx);
        }
    }

    // Fracción restante del apagado programado, para el anillo del foquito.
    fn timer_fraction(&self) -> Option<f32> {
        let due = self.timer_due?;
        let total = f32::from(self.timer_minutes) * 60.0;
        if total <= 0.0 {
            return None;
        }
        let left = due.saturating_duration_since(Instant::now()).as_secs_f32();
        Some((left / total).clamp(0.0, 1.0))
    }

    fn draw_topbar(&mut self, ui: &mut egui::Ui, compact: bool) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("WiZ Tray").size(15.0).strong().color(INK));
            if !compact {
                ui.label(RichText::new("·").size(13.0).color(BORDER_STRONG));
                ui.label(
                    RichText::new(self.preferences.backend.label())
                        .size(12.5)
                        .color(MUTED),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.connections.has(Backend::Cloud)
                    && self.preferences.backend == Backend::Cloud
                    && !has_cloud_session()
                    && ui
                        .add_sized(
                            [104.0, 30.0],
                            egui::Button::new(
                                RichText::new("Iniciar sesión").size(12.5).color(INK),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, BORDER_STRONG))
                            .corner_radius(CornerRadius::same(8)),
                        )
                        .clicked()
                {
                    self.start_action(Backend::Cloud, Action::Connect, ui.ctx());
                }
                if let Some(chosen) = backend_switch(
                    ui,
                    self.preferences.backend,
                    self.connections.has(Backend::Local),
                    self.connections.has(Backend::Cloud),
                ) {
                    self.set_backend(chosen);
                }
                ui.label(RichText::new(&self.status).size(11.5).color(MUTED));
            });
        });
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui, accent: Color32) {
        ui.horizontal(|ui| {
            for (page, label) in [
                (Page::Home, "Luz y escenas"),
                (Page::Routines, "Rutinas"),
                (Page::Connections, "Conexiones"),
            ] {
                let selected = self.page == page;
                if tab_button(ui, label, selected, accent).clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn draw_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.connections.has(self.preferences.backend) {
            self.draw_setup_prompt(ui);
            return;
        }
        // Dos columnas iguales y un único canal de 14 px cierran el ancho del panel.
        if ui.available_width() >= 720.0 {
            ui.columns(2, |columns| {
                self.draw_brightness_card(&mut columns[0], ctx);
                columns[0].add_space(14.0);
                self.draw_temperature_card(&mut columns[0], ctx);
                self.draw_color_card(&mut columns[1], ctx);
                columns[1].add_space(14.0);
                self.draw_scenes_card(&mut columns[1], ctx);
            });
        } else {
            self.draw_brightness_card(ui, ctx);
            ui.add_space(14.0);
            self.draw_temperature_card(ui, ctx);
            ui.add_space(14.0);
            self.draw_color_card(ui, ctx);
            ui.add_space(14.0);
            self.draw_scenes_card(ui, ctx);
        }
    }

    fn draw_brightness_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let accent = self.accent_color();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(display("Brillo", 15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} %", self.brightness))
                            .font(display_font(24.0))
                            .color(accent),
                    );
                });
            });
            ui.label(
                RichText::new("Intensidad de la luz")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(18.0);
            ui.spacing_mut().slider_width = ui.available_width();
            let response = ui
                .scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = accent;
                    ui.add(
                        egui::Slider::new(&mut self.brightness, 1..=100)
                            .suffix(" %")
                            .show_value(false)
                            .trailing_fill(true)
                            .handle_shape(egui::style::HandleShape::Circle),
                    )
                })
                .inner;
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                self.start_action(
                    self.preferences.backend,
                    Action::Brightness(self.brightness),
                    ctx,
                );
            }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for preset in [25, 50, 75, 100] {
                    let selected = self.brightness == preset;
                    if ui
                        .add_sized(
                            [56.0, 28.0],
                            egui::Button::new(
                                RichText::new(format!("{preset} %"))
                                    .size(12.0)
                                    .color(if selected { ON_ACCENT } else { MUTED }),
                            )
                            .fill(if selected {
                                accent
                            } else {
                                Color32::TRANSPARENT
                            })
                            .corner_radius(CornerRadius::same(8))
                            .stroke(Stroke::new(1.0, if selected { accent } else { BORDER })),
                        )
                        .clicked()
                    {
                        self.brightness = preset;
                        self.start_action(
                            self.preferences.backend,
                            Action::Brightness(preset),
                            ctx,
                        );
                    }
                }
            });
        });
    }

    fn draw_color_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        const PRESETS: &[(u8, u8, u8, &str)] = &[
            (255, 0, 0, "Rojo"),
            (255, 110, 0, "Naranja"),
            (255, 200, 0, "Amarillo"),
            (40, 200, 90, "Verde"),
            (0, 180, 220, "Cian"),
            (40, 90, 255, "Azul"),
            (150, 60, 255, "Violeta"),
            (255, 60, 150, "Rosa"),
        ];
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(display("Color", 15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "#{:02X}{:02X}{:02X}",
                            self.color[0], self.color[1], self.color[2]
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                });
            });
            ui.label(
                RichText::new("Ambiente para focos de color")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(14.0);
            let active_color = self.pilot.and_then(|pilot| pilot.color);
            ui.horizontal_wrapped(|ui| {
                for (red, green, blue, name) in PRESETS {
                    let color = Color32::from_rgb(*red, *green, *blue);
                    let active = active_color == Some((*red, *green, *blue));
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                    let response = response
                        .on_hover_text(*name)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        self.color = [*red, *green, *blue];
                        self.start_action(
                            self.preferences.backend,
                            Action::Color(*red, *green, *blue),
                            ctx,
                        );
                    }
                    let hover =
                        ui.ctx()
                            .animate_bool_with_time(response.id, response.hovered(), 0.12);
                    let painter = ui.painter();
                    if active || hover > 0.0 {
                        painter.rect_filled(
                            rect.expand(1.0 + hover * 2.0),
                            CornerRadius::same(13),
                            rgba(color, if active { 46 } else { 22 }),
                        );
                    }
                    painter.rect_filled(
                        rect.shrink(2.0 - hover * 2.0),
                        CornerRadius::same(11),
                        color,
                    );
                    if active {
                        painter.rect_stroke(
                            rect.shrink(0.5),
                            CornerRadius::same(12),
                            Stroke::new(2.0, INK),
                            StrokeKind::Inside,
                        );
                    }
                }
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Personalizado").size(12.0).color(MUTED));
                ui.color_edit_button_srgb(&mut self.color);
                if ui
                    .add(
                        egui::Button::new(RichText::new("Aplicar").size(12.0).color(INK))
                            .fill(Color32::TRANSPARENT)
                            .corner_radius(CornerRadius::same(8))
                            .stroke(Stroke::new(1.0, BORDER_STRONG)),
                    )
                    .clicked()
                {
                    self.start_action(
                        self.preferences.backend,
                        Action::Color(self.color[0], self.color[1], self.color[2]),
                        ctx,
                    );
                }
            });
        });
    }

    fn draw_setup_prompt(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(display("Conecta tu foco", 23.0));
            ui.add_space(8.0);
            ui.label("Configura una conexión local para usar la red de casa, o WiZ Cloud para controlar el foco desde cualquier lugar.");
            ui.add_space(18.0);
            if primary_button(ui, "Abrir configuración", self.accent_color()).clicked() {
                self.page = Page::Connections;
            }
        });
    }

    fn draw_temperature_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((minimum, maximum)) = self.connections.white_range(self.preferences.backend)
        else {
            card(ui, |ui| {
                ui.label(display("Temperatura", 15.0));
                ui.label("Este foco no informa un rango de blanco variable en su exportación.");
            });
            return;
        };
        let accent = self.accent_color();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(display("Temperatura", 15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} K", self.temperature))
                            .font(display_font(24.0))
                            .color(accent),
                    );
                });
            });
            ui.label(
                RichText::new("Blanco cálido o frío")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(16.0);
            self.temperature = self.temperature.clamp(minimum, maximum);
            ui.spacing_mut().slider_width = ui.available_width();
            let response = ui
                .scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = accent;
                    ui.add(
                        egui::Slider::new(&mut self.temperature, minimum..=maximum)
                            .step_by(100.0)
                            .suffix(" K")
                            .show_value(false)
                            .trailing_fill(true)
                            .handle_shape(egui::style::HandleShape::Circle),
                    )
                })
                .inner;
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                self.start_action(
                    self.preferences.backend,
                    Action::Temperature(self.temperature),
                    ctx,
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{minimum} K, cálida"))
                        .size(12.0)
                        .color(ACCENT_DARK),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{maximum} K, fría"))
                            .size(12.0)
                            .color(Color32::from_rgb(122, 178, 255)),
                    );
                });
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                for temperature in temperature_options((minimum, maximum)) {
                    let selected = self.temperature == temperature;
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{temperature} K"))
                                    .size(12.0)
                                    .color(if selected { ON_ACCENT } else { MUTED }),
                            )
                            .min_size(Vec2::new(58.0, 28.0))
                            .fill(if selected {
                                accent
                            } else {
                                Color32::TRANSPARENT
                            })
                            .corner_radius(CornerRadius::same(8))
                            .stroke(Stroke::new(1.0, if selected { accent } else { BORDER })),
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
        });
    }

    fn draw_scenes_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(display("Escenas", 15.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if self.show_all_scenes {
                                    "Ver favoritas"
                                } else {
                                    "Ver todas"
                                })
                                .size(12.0)
                                .color(MUTED),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .on_hover_text("28 escenas en total")
                        .clicked()
                    {
                        self.show_all_scenes = !self.show_all_scenes;
                    }
                });
            });
            ui.add_space(10.0);
            let show_all = self.show_all_scenes;
            let scene_count = if show_all {
                SCENES.len()
            } else {
                FAVORITE_SCENES.len()
            };
            for row in 0..scene_count.div_ceil(2) {
                ui.horizontal(|ui| {
                    for column in 0..2 {
                        let index = row * 2 + column;
                        if let Some((scene_id, name)) = scene_at(index, show_all) {
                            let active =
                                self.pilot.and_then(|pilot| pilot.scene_id) == Some(scene_id);
                            let width = ui.available_width() / (2 - column) as f32 - 7.0;
                            if scene_card(ui, name, scene_color(scene_id), active, width).clicked()
                            {
                                self.start_action(
                                    self.preferences.backend,
                                    Action::Scene(scene_id),
                                    ctx,
                                );
                            }
                        }
                    }
                });
            }
            if show_all {
                ui.label(
                    RichText::new("Algunas escenas necesitan un foco WiZ de color.")
                        .size(12.0)
                        .color(MUTED),
                );
            }
        });
    }

    fn draw_routines(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_max_width(880.0);
        card(ui, |ui| {
            ui.label(display("Apagado automático", 17.0));
            ui.label("Programa el apagado del foco después de un intervalo.");
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                let accent = self.accent_color();
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = 220.0;
                    ui.visuals_mut().selection.bg_fill = accent;
                    ui.add(
                        egui::Slider::new(&mut self.timer_minutes, 5..=180)
                            .step_by(5.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                });
                ui.label(
                    RichText::new(format!("{} min", self.timer_minutes))
                        .size(13.0)
                        .strong()
                        .color(accent),
                );
                ui.add_space(6.0);
                if self.timer_due.is_none() {
                    if primary_button(ui, "Iniciar rutina", self.accent_color()).clicked() {
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
                    .color(ACCENT_DARK),
                );
            }
            ui.add_space(12.0);
            ui.label(RichText::new("La rutina sigue activa al ocultar la ventana en la bandeja y se cancela al salir de WiZ Tray.").size(12.0).color(MUTED));
        });
        ui.add_space(14.0);
        let saved_routine = &self.preferences.daily_routine;
        let saved_status = if saved_routine.enabled {
            format!(
                "Activa, todos los días a las {:02}:{:02}. Acción: {}",
                saved_routine.minute_of_day / 60,
                saved_routine.minute_of_day % 60,
                saved_routine.action.label()
            )
        } else {
            "La rutina diaria está desactivada.".to_string()
        };
        let mut save_routine = false;
        card(ui, |ui| {
            ui.label(display("Rutina diaria", 17.0));
            ui.label("Programa una acción que se repetirá cada día.");
            ui.add_space(10.0);
            ui.checkbox(&mut self.routine_draft.enabled, "Activar rutina diaria");
            ui.horizontal(|ui| {
                ui.label("Hora local");
                let mut hour = self.routine_draft.minute_of_day / 60;
                let mut minute = self.routine_draft.minute_of_day % 60;
                ui.add(egui::DragValue::new(&mut hour).range(0..=23).suffix(" h"));
                ui.label(":");
                ui.add(
                    egui::DragValue::new(&mut minute)
                        .range(0..=59)
                        .suffix(" min"),
                );
                self.routine_draft.minute_of_day = hour * 60 + minute;
            });
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut self.routine_draft.action, RoutineAction::Off, "Apagar");
                ui.radio_value(
                    &mut self.routine_draft.action,
                    RoutineAction::On,
                    "Encender",
                );
                ui.radio_value(
                    &mut self.routine_draft.action,
                    RoutineAction::Daylight,
                    "Encender · Luz de día",
                );
            });
            ui.add_space(4.0);
            ui.label(RichText::new(saved_status).size(12.0).color(MUTED));
            ui.label(RichText::new("Se ejecuta mientras WiZ Tray esté abierta en la bandeja. Cloud necesita una sesión iniciada.").size(12.0).color(MUTED));
            ui.add_space(8.0);
            save_routine = ui
                .add(
                    egui::Button::new(
                        RichText::new("Guardar rutina diaria")
                            .strong()
                            .color(ON_ACCENT),
                    )
                    .fill(ACCENT)
                    .corner_radius(CornerRadius::same(8))
                    .stroke(Stroke::NONE),
                )
                .clicked();
        });
        if save_routine {
            self.save_daily_routine();
        }
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.label(display("Accesos rápidos", 15.0));
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

    fn save_daily_routine(&mut self) {
        let mut preferences = self.preferences.clone();
        let mut routine = self.routine_draft.clone();
        routine.last_run_day = preferences.daily_routine.last_run_day;
        preferences.daily_routine = routine;
        if let Err(error) = save_preferences(&preferences) {
            self.last_error = Some(error.clone());
            self.set_status(&error);
            return;
        }
        self.preferences = preferences;
        self.draft.daily_routine = self.preferences.daily_routine.clone();
        self.routine_draft = self.preferences.daily_routine.clone();
        self.last_error = None;
        self.set_status("Rutina diaria guardada.");
    }

    fn draw_connections(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_max_width(880.0);
        let accent = self.accent_color();
        ui.horizontal(|ui| {
            if tab_button(
                ui,
                "Local (misma red)",
                self.setup_tab == Backend::Local,
                accent,
            )
            .clicked()
            {
                self.setup_tab = Backend::Local;
            }
            if tab_button(
                ui,
                "WiZ Cloud (remoto)",
                self.setup_tab == Backend::Cloud,
                accent,
            )
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
            ui.label(display("Conexión local", 17.0));
            ui.label("Directo por Wi-Fi, sin cuenta ni nube.");
            ui.add_space(16.0);
            ui.label(RichText::new("1. Activa el control local").strong());
            ui.label("En WiZ: Ajustes → Seguridad → activa la comunicación local.");
            ui.add_space(8.0);
            ui.label(RichText::new("2. Encuentra el foco").strong());
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.discovery_running,
                        egui::Button::new("Buscar focos en mi red"),
                    )
                    .clicked()
                {
                    self.start_discovery(ctx);
                }
                if self.discovery_running {
                    ui.spinner();
                    ui.label(RichText::new("Escaneando la red…").size(12.0).color(MUTED));
                }
            });
            if !self.discovered.is_empty() {
                ui.add_space(6.0);
                let mut chosen = None;
                for light in &self.discovered {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(light.ip.to_string()).monospace().size(12.0));
                        ui.label(
                            RichText::new(format!("MAC {}", short_mac(&light.mac)))
                                .size(12.0)
                                .color(MUTED),
                        );
                        ui.label(
                            RichText::new(if light.state { "Encendida" } else { "Apagada" })
                                .size(12.0)
                                .color(if light.state { SUCCESS } else { MUTED }),
                        );
                        if ui.small_button("Usar y conectar").clicked() {
                            chosen = Some(light.clone());
                        }
                    });
                }
                if let Some(light) = chosen {
                    self.use_discovered_light(&light);
                    match self.save_local_setup() {
                        Ok(()) => self.start_action(Backend::Local, Action::Refresh, ctx),
                        Err(error) => self.last_error = Some(error),
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("3. Datos del foco").strong());
            ui.horizontal(|ui| {
                ui.label("IP local");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.local_ip)
                        .hint_text("192.168.1.25")
                        .desired_width(150.0),
                );
                ui.label("MAC");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.local_mac)
                        .hint_text("AA:BB:CC:DD:EE:FF")
                        .desired_width(160.0),
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
            ui.add_space(8.0);
            ui.label(RichText::new("4. Token de firma (opcional)").strong());
            ui.horizontal(|ui| {
                let displayed = if self.draft.local_export.is_empty() {
                    "Sin export: comandos sin firma".to_string()
                } else {
                    Path::new(&self.draft.local_export)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Export WiZ seleccionado")
                        .to_string()
                };
                ui.label(RichText::new(displayed).size(12.0).color(MUTED));
                if ui.button("Elegir export…").clicked()
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
                if !self.export_devices.is_empty() {
                    let devices = self.export_devices.clone();
                    let mut selected_mac = normalize_mac(&self.draft.local_mac);
                    egui::ComboBox::from_id_salt("export_device_pick")
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
                    if selected_mac != normalize_mac(&self.draft.local_mac) {
                        self.draft.local_mac = selected_mac;
                        if let Some(device) = devices
                            .iter()
                            .find(|device| device.mac == self.draft.local_mac)
                        {
                            self.draft.device_name = device.name.clone();
                        }
                    }
                }
            });
            let export_has_light = self
                .export_devices
                .iter()
                .any(|device| device.mac == normalize_mac(&self.draft.local_mac));
            ui.label(
                RichText::new(if self.draft.local_export.trim().is_empty() {
                    "Sin token: comandos sin firma. Suele funcionar; si el foco los rechaza, importa el export."
                } else if export_has_light {
                    "Export cargado: los comandos se firman con su clave."
                } else {
                    "El export no incluye este foco; se usará sin token."
                })
                .size(12.0)
                .color(MUTED),
            );
            ui.label("El export tiene una clave privada. Se queda en tu equipo: no lo compartas.");
            ui.add_space(12.0);
            if let Some(error) = &self.last_error {
                ui.colored_label(ERROR, error);
            }
            if ui
                .add_sized(
                    [170.0, 42.0],
                    egui::Button::new(RichText::new("Guardar y probar").strong().color(ON_ACCENT))
                        .fill(ACCENT)
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
                        .size(12.0)
                        .color(SUCCESS),
                );
            }
            ui.collapsing("¿Hace falta el token? ¿Cómo se encuentra el foco?", |ui| {
                ui.label("La app habla UDP 38899 con el foco. El token es la clave HMAC del export: los firmwares nuevos suelen exigirla; la mayoría de los domésticos, no.");
                ui.label("La búsqueda escanea tu subred /24 con getPilot y muestra IP + MAC. Si el foco no aparece, escribe ambos a mano.");
                ui.label("Todo ocurre en tu red local: nada se envía a WiZ ni a Google.");
            });
        });
    }

    fn use_discovered_light(&mut self, light: &DiscoveredLight) {
        self.draft.local_ip = light.ip.to_string();
        self.draft.local_mac = light.mac.clone();
        self.draft.device_name = self
            .export_devices
            .iter()
            .find(|device| device.mac == light.mac)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| format!("Foco WiZ ({})", short_mac(&light.mac)));
        self.last_error = None;
        self.set_status(&format!("Foco seleccionado: {}", self.draft.device_name));
    }

    fn save_local_setup(&mut self) -> Result<(), String> {
        let local = build_local_config(&self.draft)?.ok_or_else(|| {
            "Escribe la IP y la MAC del foco, o usa Buscar focos en mi red.".to_string()
        })?;
        if self.draft.device_name.trim().is_empty() {
            self.draft.device_name = self
                .export_devices
                .iter()
                .find(|device| device.mac == self.draft.local_mac)
                .map(|device| device.name.clone())
                .unwrap_or_else(|| format!("Foco WiZ ({})", short_mac(&self.draft.local_mac)));
        }
        let backend_changed = self.preferences.backend != Backend::Local;
        self.draft.backend = Backend::Local;
        self.draft.daily_routine = self.preferences.daily_routine.clone();
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
        if !self.cloud_discovery_started && self.draft.cloud_mac.trim().is_empty() {
            self.cloud_discovery_started = true;
            self.start_discovery(ctx);
        }
        card(ui, |ui| {
            ui.label(display("WiZ Cloud", 17.0));
            ui.label(
                "Control remoto: este equipo debe seguir encendido, con internet y la app abierta.",
            );
            ui.add_space(10.0);
            ui.label("La API doméstica de WiZ no es pública. Google confirma tu cuenta, pero WiZ pide credenciales OAuth privadas que este repo no incluye.");
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                let checks = [
                    (
                        "Foco y hogar",
                        !self.draft.cloud_device_id.trim().is_empty()
                            && !self.draft.cloud_home_id.trim().is_empty()
                            && !self.draft.cloud_mac.trim().is_empty(),
                    ),
                    ("Credenciales", self.cloud_credentials.is_some()),
                    ("Sesión", has_cloud_session()),
                ];
                for (label, done) in checks {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                        ui.painter().circle_filled(
                            rect.center(),
                            3.4,
                            if done { SUCCESS } else { BORDER_STRONG },
                        );
                        ui.label(RichText::new(label).size(12.0).color(if done {
                            SUCCESS
                        } else {
                            MUTED
                        }));
                    });
                }
            });
            ui.add_space(14.0);
            let accent = self.accent_color();
            ui.horizontal_wrapped(|ui| {
                for (step, label) in ["1 · Foco", "2 · Acceso", "3 · Conectar"]
                    .iter()
                    .enumerate()
                {
                    if tab_button(ui, label, self.cloud_setup_step == step, accent).clicked() {
                        self.cloud_setup_step = step;
                    }
                }
            });
            ui.add_space(12.0);
            if self.cloud_setup_step == 0 {
                ui.label(RichText::new("Encuentra tu foco").strong());
                ui.label("Buscamos la MAC del foco en tu Wi-Fi. Elige el dispositivo correcto; la búsqueda no obtiene los IDs de Cloud.");
                if self.discovery_running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Buscando focos en la red…");
                    });
                } else if primary_button(ui, "Buscar de nuevo en mi red", accent).clicked() {
                    self.start_discovery(ctx);
                }
                for light in self.discovered.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Foco · {} · {}", light.ip, light.mac));
                        if primary_button(ui, "Usar este foco", accent).clicked() {
                            let mac = normalize_mac(&light.mac);
                            if mac.len() == 12 && mac.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                                self.draft.cloud_mac = mac.clone();
                                if let Some(device) = self
                                    .export_devices
                                    .iter()
                                    .find(|device| normalize_mac(&device.mac) == mac)
                                    .cloned()
                                {
                                    self.use_cloud_export_device(&device);
                                }
                            } else {
                                self.last_error = Some(
                                    "El foco devolvió una MAC inválida. Prueba otro dispositivo."
                                        .into(),
                                );
                            }
                        }
                    });
                }
                ui.collapsing("No aparece · Ayuda desde el móvil", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (number, title, detail) in [
                        ("01", "Misma Wi-Fi", "Conecta el móvil, el foco y el PC a tu red doméstica; evita la red de invitados."),
                        ("02", "App WiZ", "Abre WiZ y comprueba que puedes controlar el foco. Activa la comunicación local en Ajustes → Seguridad, si aparece."),
                        ("03", "Vuelve a buscar", "Reintenta aquí. Si sigue sin aparecer, consulta la información del foco en WiZ o en tu router para localizar su MAC."),
                    ] {
                        egui::Frame::new().fill(SURFACE_SOFT).corner_radius(10).inner_margin(12).show(ui, |ui| {
                            ui.set_max_width(210.0);
                            ui.label(RichText::new(number).size(24.0).color(ACCENT_DARK));
                            ui.label(RichText::new(title).strong());
                            ui.label(detail);
                        });
                    }
                });
                ui.label("Si ya tienes un export compatible, selecciónalo abajo desde este equipo. La disponibilidad de información y exportación varía según la versión de WiZ.");
            });
                ui.add_space(12.0);
                ui.label(RichText::new("Identifica el foco y el hogar").strong());
                ui.label("Importa el export WiZ para rellenar los datos, o escribe el ID del foco, el ID del hogar y la MAC.");
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
                        egui::ComboBox::from_id_salt("export_device_pick")
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
                    ui.label(RichText::new("opcional").size(12.0).color(MUTED));
                });
                ui.add_space(14.0);
                let identity_ready = self
                    .draft
                    .cloud_device_id
                    .parse::<u64>()
                    .is_ok_and(|id| id > 0)
                    && self
                        .draft
                        .cloud_home_id
                        .parse::<u64>()
                        .is_ok_and(|id| id > 0)
                    && {
                        let mac = normalize_mac(&self.draft.cloud_mac);
                        mac.len() == 12 && mac.bytes().all(|byte| byte.is_ascii_hexdigit())
                    };
                if ui
                    .add_enabled(identity_ready, egui::Button::new("Continuar · Acceso"))
                    .clicked()
                {
                    self.cloud_setup_step = 1;
                }
            }
            if self.cloud_setup_step == 1 {
                ui.label(RichText::new("Prepara el acceso seguro").strong());
                ui.label(
                "WiZ: Client ID y secreto privados. Google: su Client ID; el secreto es opcional.",
            );
                ui.collapsing("Guía: cómo obtener estas credenciales", |ui| {
                ui.label(RichText::new("1. Google").strong());
                ui.label("Crea un cliente OAuth de tipo «App de escritorio». No hace falta habilitar APIs ni registrar la URL de retorno: la app usa el loopback 127.0.0.1.");
                ui.horizontal(|ui| {
                    ui.hyperlink_to(
                        "Abrir Google Cloud Console",
                        "https://console.cloud.google.com/apis/credentials",
                    );
                    ui.hyperlink_to(
                        "Pantalla de consentimiento",
                        "https://console.cloud.google.com/apis/credentials/consent",
                    );
                });
                ui.label("Si el proyecto está en modo prueba, agrega tu cuenta como usuario de prueba.");
                ui.add_space(8.0);
                ui.label(RichText::new("2. WiZ").strong());
                ui.label("Son los de la app oficial de WiZ. WiZ no los publica: se obtienen del cliente oficial en un dispositivo propio. No los compartas.");
                ui.add_space(8.0);
                ui.label(RichText::new("3. Guarda y autoriza").strong());
                ui.label("Guarda Cloud, pulsa Iniciar sesión con Google y vuelve sola a la app. La sesión queda cifrada con DPAPI.");
                ui.label(
                    RichText::new("Errores típicos: redirect_uri_mismatch → usa un cliente Desktop; invalid_client → ID o secreto incorrectos; access_denied → falta tu usuario de prueba.")
                        .size(12.0)
                        .color(MUTED),
                );
            });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Credenciales de OAuth").strong());
                    if self.cloud_credentials.is_some() && !self.replace_credentials {
                        ui.label(
                            RichText::new("Protegidas con Windows DPAPI")
                                .size(12.0)
                                .color(SUCCESS),
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
                    ui.label(RichText::new("Al guardar, estos valores se cifran con DPAPI y quedan ligados a tu perfil de Windows.").size(12.0).color(MUTED));
                }
                ui.add_space(12.0);
                if let Some(error) = &self.last_error {
                    ui.colored_label(ERROR, error);
                    if let Some(hint) = cloud_error_hint(error) {
                        ui.label(RichText::new(hint).size(12.0).color(MUTED));
                    }
                }
                if primary_button(ui, "Continuar · Conectar", accent).clicked() {
                    self.cloud_setup_step = 2;
                }
            }
            if self.cloud_setup_step == 2 {
                ui.label(RichText::new("Conecta y comprueba el foco").strong());
                ui.label("El navegador abre Google; después comprobamos que WiZ Cloud puede leer tu foco.");
                if let Some(error) = &self.last_error {
                    ui.colored_label(ERROR, error);
                    if let Some(hint) = cloud_error_hint(error) {
                        ui.label(hint);
                    }
                }
                ui.label("Guarda y autoriza con Google. La sesión vive en este perfil de Windows.");
                ui.horizontal(|ui| {
                    if primary_button(ui, "Guardar y conectar", accent).clicked() {
                        match self.save_cloud_setup() {
                            Ok(()) => self.start_action(Backend::Cloud, Action::Connect, ctx),
                            Err(error) => self.last_error = Some(error),
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
                    if self.connections.has(Backend::Cloud)
                        && ui.button("Probar conexión").clicked()
                    {
                        self.set_backend(Backend::Cloud);
                        self.page = Page::Home;
                        let action = if has_cloud_session() {
                            Action::Refresh
                        } else {
                            Action::Connect
                        };
                        self.start_action(Backend::Cloud, action, ctx);
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
                        .size(12.0)
                        .color(SUCCESS),
                    );
                }
            }
        });
    }

    fn save_cloud_setup(&mut self) -> Result<(), String> {
        let cloud_white_range = parse_optional_range(&self.cloud_minimum, &self.cloud_maximum)?;
        let mut draft = self.draft.clone();
        draft.backend = Backend::Cloud;
        draft.cloud_white_range = cloud_white_range;
        draft.daily_routine = self.preferences.daily_routine.clone();
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
        ui.painter().rect_filled(ui.max_rect(), 0, CANVAS);
        egui::Frame::new()
            .fill(Color32::TRANSPARENT)
            .show(ui, |ui| self.draw(ui, &ctx));
    }
}

fn apply_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CANVAS;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = SURFACE_SOFT;
    visuals.faint_bg_color = SURFACE_SOFT;
    visuals.text_edit_bg_color = Some(SURFACE_SOFT);
    visuals.override_text_color = Some(INK);
    visuals.weak_text_color = Some(MUTED);
    visuals.hyperlink_color = ACCENT_DARK;
    visuals.selection.bg_fill = ACCENT_PALE;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.inactive.bg_fill = SURFACE_SOFT;
    visuals.widgets.inactive.weak_bg_fill = SURFACE_SOFT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 56);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 56);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.4, INK);
    visuals.widgets.active.bg_fill = ACCENT_PALE;
    visuals.widgets.active.weak_bg_fill = ACCENT_PALE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.4, INK);
    visuals.widgets.open.bg_fill = SURFACE_SOFT;
    visuals.widgets.open.weak_bg_fill = SURFACE_SOFT;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.open.corner_radius = CornerRadius::same(8);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals(visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(14.0, 10.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        style.spacing.button_padding = Vec2::new(15.0, 9.0);
        style.spacing.scroll.active_handle_opacity = 0.28;
        style.spacing.scroll.interact_handle_opacity = 0.45;
    });
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut body_loaded = false;
    if let Ok(bytes) = fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts.font_data.insert(
            "wiz-body".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "wiz-body".to_owned());
        body_loaded = true;
    }
    let mut display_family = Vec::new();
    if let Ok(bytes) = fs::read(r"C:\Windows\Fonts\seguisb.ttf") {
        fonts.font_data.insert(
            "wiz-display".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        display_family.push("wiz-display".to_owned());
    }
    display_family.extend(
        fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("display".into()), display_family);
    if !body_loaded {
        // La app es Windows-only; si faltara Segoe UI, egui usa su fuente por defecto.
    }
    ctx.set_fonts(fonts);
}

fn display_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("display".into()))
}

fn display(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(display_font(size)).color(INK)
}

fn tab_button(ui: &mut egui::Ui, label: &str, selected: bool, accent: Color32) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(13.5).color(if selected {
            INK
        } else {
            MUTED
        }))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(0.0, 30.0)),
    );
    if selected {
        let rect = response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 2.0, rect.bottom() - 2.0),
                egui::pos2(rect.right() - 2.0, rect.bottom()),
            ),
            CornerRadius::same(2),
            accent,
        );
    }
    response
}

/// Botón de acción principal: relleno de acento y texto de contraste.
fn primary_button(ui: &mut egui::Ui, label: &str, accent: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(ON_ACCENT))
            .fill(accent)
            .corner_radius(CornerRadius::same(10))
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(0.0, 38.0)),
    )
}

fn backend_switch(
    ui: &mut egui::Ui,
    current: Backend,
    has_local: bool,
    has_cloud: bool,
) -> Option<Backend> {
    let mut chosen = None;
    egui::Frame::new()
        .fill(SURFACE_SOFT)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(3, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (backend, label, available) in [
                    (Backend::Local, "Casa", has_local),
                    (Backend::Cloud, "Cloud", has_cloud),
                ] {
                    if !available {
                        continue;
                    }
                    let selected = current == backend;
                    if ui
                        .add_sized(
                            [58.0, 24.0],
                            egui::Button::new(RichText::new(label).size(12.0).color(if selected {
                                INK
                            } else {
                                MUTED
                            }))
                            .fill(if selected {
                                SURFACE
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(7)),
                        )
                        .clicked()
                    {
                        chosen = Some(backend);
                    }
                }
            });
        });
    chosen
}

fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.with_layout(Layout::top_down(Align::Min), add_contents);
        });
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.vertical(|ui| {
        ui.label(label);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(value).hint_text(hint),
        );
    });
}

fn secret_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.label(label);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(value).password(true),
        );
    });
}

fn hero_stat(ui: &mut egui::Ui, label: &str, value: &str, accent: Color32) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.5).color(MUTED));
        ui.add_space(1.0);
        ui.label(
            RichText::new(value)
                .size(15.0)
                .strong()
                .color(if value == "—" { MUTED } else { accent }),
        );
    });
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.6, color);
}

fn paint_signal(ui: &mut egui::Ui, rssi: Option<i8>) {
    let Some(rssi) = rssi else {
        return;
    };
    let active_bars = match rssi {
        value if value >= -55 => 4,
        value if value >= -65 => 3,
        value if value >= -75 => 2,
        _ => 1,
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 13.0), Sense::hover());
    let painter = ui.painter();
    for bar in 0..4 {
        let height = 4.0 + bar as f32 * 3.0;
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + bar as f32 * 5.0, rect.bottom() - height),
            egui::pos2(rect.left() + bar as f32 * 5.0 + 3.0, rect.bottom()),
        );
        painter.rect_filled(
            bar_rect,
            CornerRadius::same(1),
            if bar < active_bars { SUCCESS } else { BORDER },
        );
    }
    response.on_hover_text(format!("Señal Wi-Fi: {rssi} dBm"));
}

fn scene_card(
    ui: &mut egui::Ui,
    name: &str,
    color: Color32,
    active: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 54.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), 0.12);
    let swatch = egui::Rect::from_min_size(rect.min, Vec2::new(width, 34.0));
    let painter = ui.painter();
    painter.rect_filled(swatch, CornerRadius::same(10), mix(color, INK, 0.10));
    painter.rect_filled(
        egui::Rect::from_min_size(swatch.min + Vec2::new(0.0, 10.0), Vec2::new(width, 24.0)),
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 10,
            se: 10,
        },
        rgba(Color32::BLACK, 46),
    );
    if active {
        painter.rect_stroke(
            swatch,
            CornerRadius::same(10),
            Stroke::new(2.0, INK),
            StrokeKind::Inside,
        );
    }
    let label = egui::pos2(swatch.left(), swatch.bottom() + 11.0);
    painter.text(
        label,
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(11.5),
        if active {
            INK
        } else {
            mix(MUTED, INK, 0.2 + hover * 0.45)
        },
    );
    if active {
        painter.circle_filled(egui::pos2(swatch.right() - 6.0, label.y), 3.0, color);
    }
    response
}

fn paint_bulb(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    halo: f32,
    color: Color32,
    on: bool,
    pulse: f32,
    brightness: f32,
    timer: Option<f32>,
) {
    let center = egui::pos2(rect.center().x, rect.top() + halo + radius);
    let metal = if on {
        mix(color, INK, 0.68)
    } else {
        Color32::from_rgb(72, 72, 82)
    };

    if on {
        let layers = ((halo / 2.5).round() as usize).max(4);
        for layer in (1..=layers).rev() {
            let spread = radius + halo * layer as f32 / layers as f32;
            painter.circle_filled(
                center,
                spread,
                rgba(color, (4.0 * pulse * brightness) as u8),
            );
        }
    }

    let neck_width = radius * 0.70;
    let neck = egui::Rect::from_center_size(
        egui::pos2(center.x, center.y + radius * 0.80),
        Vec2::new(neck_width, radius * 0.58),
    );
    painter.rect_filled(neck, 4, metal);
    let base_y = neck.bottom() + 3.0;
    for row in 0..3 {
        let width = neck_width * (0.96 - row as f32 * 0.15);
        painter.line_segment(
            [
                egui::pos2(center.x - width / 2.0, base_y + row as f32 * 5.0),
                egui::pos2(center.x + width / 2.0, base_y + row as f32 * 5.0),
            ],
            Stroke::new(2.4, mix(metal, INK, 0.30)),
        );
    }
    painter.circle_filled(egui::pos2(center.x, base_y + 14.0), 4.2, metal);

    painter.circle_filled(
        center,
        radius,
        if on {
            mix(color, INK, 0.10)
        } else {
            Color32::from_rgb(44, 44, 52)
        },
    );
    if on {
        // Núcleo caliente: el vidrio se aclara hacia el centro.
        painter.circle_filled(center, radius * 0.76, mix(color, INK, 0.22));
        painter.circle_filled(center, radius * 0.50, mix(color, INK, 0.42));
        painter.circle_filled(center, radius * 0.24, mix(color, INK, 0.62));
        painter.circle_filled(
            egui::pos2(center.x - radius * 0.34, center.y - radius * 0.38),
            radius * 0.16,
            rgba(INK, 34),
        );
        // Filamento: el bucle de una bombilla incandescente.
        let filament = egui::pos2(center.x, center.y + radius * 0.22);
        let loop_radius = radius * 0.15;
        let loop_points: Vec<egui::Pos2> = (0..=16)
            .map(|step| {
                let angle = step as f32 / 16.0 * std::f32::consts::PI;
                egui::pos2(
                    filament.x - angle.cos() * loop_radius,
                    filament.y - angle.sin() * loop_radius * 1.2,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            loop_points,
            Stroke::new(2.4, rgba(INK, 245)),
        ));
    } else {
        painter.circle_stroke(center, radius, Stroke::new(1.2, BORDER_STRONG));
    }

    // Charco de luz sobre la mesa: capas elípticas que acumulan un degradado suave.
    if on {
        let pool_y = rect.bottom() - 6.0;
        for layer in 0..5 {
            let t = 1.0 - layer as f32 / 5.0;
            let spread = radius * (0.55 + t * 1.15);
            let pool: Vec<egui::Pos2> = (0..30)
                .map(|index| {
                    let angle = index as f32 / 30.0 * std::f32::consts::TAU;
                    egui::pos2(
                        center.x + angle.cos() * spread,
                        pool_y + angle.sin() * spread * 0.20,
                    )
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(
                pool,
                rgba(color, (13.0 * pulse * brightness) as u8),
                Stroke::NONE,
            ));
        }
    }

    // Anillo del apagado programado.
    if let Some(fraction) = timer {
        let ring = radius * 1.32;
        let start = -std::f32::consts::FRAC_PI_2;
        let end = start + fraction * std::f32::consts::TAU;
        let arc: Vec<egui::Pos2> = (0..=48)
            .map(|step| {
                let angle = start + (end - start) * step as f32 / 48.0;
                egui::pos2(center.x + angle.cos() * ring, center.y + angle.sin() * ring)
            })
            .collect();
        painter.add(egui::Shape::line(arc, Stroke::new(2.0, rgba(INK, 130))));
    }
}

// WiZ reports the scene, not its instantaneous RGB animation; this is a representative preview.
fn preview_light_color(pilot: Option<Pilot>, time: f32, animate: bool) -> Color32 {
    let Some(pilot) = pilot.filter(|pilot| pilot.state) else {
        return SURFACE_SOFT;
    };
    if let Some(scene) = pilot.scene_id.filter(|scene| *scene != 0) {
        let base = scene_color(scene);
        if animate && matches!(scene, 1..=10 | 17..=28) {
            let next = match scene {
                1 | 23 => rgb(65, 200, 174),
                5 | 6 | 22 => rgb(250, 180, 60),
                7 | 24 | 25 => rgb(166, 206, 70),
                3 | 9 | 10 => rgb(207, 84, 113),
                _ => scene_color(1 + ((time / 3.0) as u16 % 8)),
            };
            return mix(base, next, 0.5 + 0.5 * (time * 0.7).sin());
        }
        return base;
    }
    if let Some((red, green, blue)) = pilot.color {
        return rgb(red, green, blue);
    }
    let temperature = f32::from(pilot.temp.unwrap_or(4000));
    mix(
        rgb(255, 180, 95),
        rgb(190, 220, 255),
        (temperature - 2200.0) / 4300.0,
    )
}

fn scene_color(scene_id: u16) -> Color32 {
    match scene_id {
        PREFERRED_SCENE => rgb(232, 172, 52),
        11 => rgb(232, 156, 74),
        13 => rgb(116, 166, 204),
        15 => rgb(82, 156, 130),
        16 => rgb(140, 118, 198),
        14 => rgb(74, 92, 168),
        1 => rgb(52, 138, 189),
        2 => rgb(224, 96, 132),
        3 => rgb(240, 133, 69),
        4 => rgb(198, 84, 206),
        5 => rgb(214, 105, 48),
        6 => rgb(196, 148, 96),
        7 => rgb(72, 150, 91),
        8 => rgb(210, 156, 196),
        9 => rgb(243, 188, 92),
        10 => rgb(70, 82, 140),
        17 => rgb(96, 165, 196),
        18 => rgb(104, 118, 168),
        19 => rgb(98, 168, 108),
        20 => rgb(150, 196, 90),
        21 => rgb(240, 178, 56),
        22 => rgb(196, 120, 60),
        23 => rgb(42, 108, 160),
        24 => rgb(48, 140, 96),
        25 => rgb(120, 200, 140),
        26 => rgb(170, 70, 200),
        27 => rgb(200, 60, 70),
        28 => rgb(240, 130, 40),
        _ => MUTED,
    }
}

fn mix(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 * (1.0 - amount) + b as f32 * amount).round() as u8;
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
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

fn local_day_and_minute() -> (u32, u16) {
    let mut now = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut now) };
    let day = u32::from(now.wYear) * 10_000 + u32::from(now.wMonth) * 100 + u32::from(now.wDay);
    let minute = u16::from(now.wHour) * 60 + u16::from(now.wMinute);
    (day, minute)
}

fn routine_action_due(routine: &DailyRoutine, today: u32, minute: u16) -> Option<Action> {
    (routine.enabled && routine.minute_of_day == minute && routine.last_run_day != Some(today))
        .then(|| routine.action.action())
}

fn scene_at(index: usize, show_all: bool) -> Option<(u16, &'static str)> {
    let scene_id = if show_all {
        SCENES.get(index)?.0
    } else {
        *FAVORITE_SCENES.get(index)?
    };
    SCENES
        .iter()
        .find(|(candidate, _)| *candidate == scene_id)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::{
        DailyRoutine, FAVORITE_SCENES, Preferences, RoutineAction, SCENES, daily_routine_from_json,
        preferences_from_json, preferences_json, routine_action_due, scene_at,
    };

    #[test]
    fn light_preview_tracks_color_scene_and_reduced_motion() {
        let mut pilot = wiz_tray::Pilot {
            state: true,
            scene_id: None,
            temp: None,
            dimming: Some(65),
            color: Some((20, 80, 160)),
            rssi: None,
        };
        assert_eq!(
            super::preview_light_color(Some(pilot), 0.0, true),
            super::rgb(20, 80, 160)
        );
        pilot.scene_id = Some(1);
        assert_ne!(
            super::preview_light_color(Some(pilot), 0.0, true),
            super::preview_light_color(Some(pilot), 2.0, true)
        );
        assert_eq!(
            super::preview_light_color(Some(pilot), 0.0, false),
            super::preview_light_color(Some(pilot), 2.0, false)
        );
        pilot.state = false;
        assert_eq!(
            super::preview_light_color(Some(pilot), 0.0, true),
            super::SURFACE_SOFT
        );
    }

    #[test]
    fn scene_catalog_keeps_favorites_and_optional_modes() {
        assert_eq!(scene_at(0, false), Some((12, "Luz de día")));
        assert_eq!(scene_at(9, true), Some((4, "Fiesta")));
        assert_eq!(scene_at(27, true), Some((28, "Halloween")));
        assert_eq!(scene_at(SCENES.len(), true), None);
        assert_eq!(FAVORITE_SCENES.len(), 6);
        assert!(SCENES.iter().all(|(id, _)| {
            SCENES
                .iter()
                .filter(|(candidate, _)| candidate == id)
                .count()
                == 1
        }));
    }

    #[test]
    fn daily_routine_is_persisted_and_rejects_invalid_times() {
        let mut preferences = Preferences::default();
        preferences.daily_routine = DailyRoutine {
            enabled: true,
            minute_of_day: 7 * 60 + 30,
            action: RoutineAction::Daylight,
            last_run_day: Some(20261016),
        };
        let saved = preferences_json(&preferences);
        assert_eq!(
            preferences_from_json(&saved).unwrap().daily_routine,
            preferences.daily_routine
        );

        let mut legacy = preferences_json(&Preferences::default());
        legacy.as_object_mut().unwrap().remove("daily_routine");
        assert_eq!(
            preferences_from_json(&legacy).unwrap().daily_routine,
            DailyRoutine::default()
        );

        let invalid = serde_json::json!({
            "enabled": true,
            "minute_of_day": 1440,
            "action": "off"
        });
        assert!(daily_routine_from_json(Some(&invalid)).is_err());
    }

    #[test]
    fn daily_routine_runs_once_at_its_local_minute() {
        let routine = DailyRoutine {
            enabled: true,
            minute_of_day: 7 * 60 + 30,
            action: RoutineAction::Daylight,
            last_run_day: None,
        };
        assert!(matches!(
            routine_action_due(&routine, 20261016, 450),
            Some(super::Action::Daylight)
        ));
        assert!(routine_action_due(&routine, 20261016, 451).is_none());
        let already_run = DailyRoutine {
            last_run_day: Some(20261016),
            ..routine
        };
        assert!(routine_action_due(&already_run, 20261016, 450).is_none());
    }
}

fn pilot_status(pilot: Pilot) -> String {
    if !pilot.state {
        return "Apagada".to_string();
    }
    let scene = scene_name(pilot.scene_id);
    let base = if scene != "Modo actual" {
        scene.to_string()
    } else {
        pilot
            .temp
            .map(|temperature| format!("{temperature} K"))
            .unwrap_or_else(|| "Encendida".to_string())
    };
    pilot
        .dimming
        .filter(|dimming| *dimming < 100)
        .map(|dimming| format!("{base} · {dimming} %"))
        .unwrap_or(base)
}

fn cloud_error_hint(error: &str) -> Option<&'static str> {
    let error = error.to_lowercase();
    if error.contains("redirect_uri_mismatch") {
        Some("Google rechazó la URL de retorno: crea el cliente OAuth como «App de escritorio».")
    } else if error.contains("invalid_client") || error.contains("unauthorized_client") {
        Some("Revisa el Client ID y el secreto: no coinciden o el cliente no es de tipo Desktop.")
    } else if error.contains("access_denied") {
        Some("Agrega tu cuenta de Google como usuario de prueba en la pantalla de consentimiento.")
    } else if error.contains("faltan las credenciales") {
        Some("Guarda primero las credenciales de OAuth en esta misma pantalla.")
    } else {
        None
    }
}

fn short_mac(mac: &str) -> String {
    let mac = normalize_mac(mac);
    mac.get(mac.len().saturating_sub(4)..)
        .unwrap_or_default()
        .to_string()
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

fn rgba(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
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
        },
        "daily_routine": {
            "enabled": preferences.daily_routine.enabled,
            "minute_of_day": preferences.daily_routine.minute_of_day,
            "action": preferences.daily_routine.action.as_str(),
            "last_run_day": preferences.daily_routine.last_run_day,
        }
    })
}

fn daily_routine_from_json(value: Option<&Value>) -> Result<DailyRoutine, String> {
    let Some(value) = value else {
        return Ok(DailyRoutine::default());
    };
    let enabled = value
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "La rutina diaria guardada debe indicar si está activa.".to_string())?;
    let minute_of_day = value
        .get("minute_of_day")
        .and_then(Value::as_u64)
        .and_then(|minute| u16::try_from(minute).ok())
        .filter(|minute| *minute < 24 * 60)
        .ok_or_else(|| "La hora guardada de la rutina diaria no es válida.".to_string())?;
    let action = RoutineAction::from_str(value.get("action").and_then(Value::as_str))?;
    let last_run_day = match value.get("last_run_day") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|day| u32::try_from(day).ok())
                .filter(|day| (19700101..=99991231).contains(day))
                .ok_or_else(|| {
                    "La fecha de última ejecución de la rutina no es válida.".to_string()
                })?,
        ),
    };
    Ok(DailyRoutine {
        enabled,
        minute_of_day,
        action,
        last_run_day,
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
        daily_routine: daily_routine_from_json(value.get("daily_routine"))?,
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
    if preferences.local_ip.trim().is_empty() || preferences.local_mac.trim().is_empty() {
        return Ok(None);
    }
    let ip = preferences
        .local_ip
        .trim()
        .parse::<Ipv4Addr>()
        .map_err(|_| "Escribe una dirección IPv4 local válida para el foco.".to_string())?;
    let mac = normalize_mac(&preferences.local_mac);
    if mac.len() != 12 || !mac.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("La MAC del foco debe tener 12 caracteres hexadecimales.".to_string());
    }
    // Sin export la clave queda vacía y los comandos se envían sin firma.
    let mut key = Vec::new();
    let mut white_range = None;
    if !preferences.local_export.trim().is_empty() {
        let bytes = fs::read(&preferences.local_export)
            .map_err(|_| "No pude abrir el export WiZ seleccionado.".to_string())?;
        let home: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "El export WiZ seleccionado no es JSON válido.".to_string())?;
        if let Some(device) = parse_export_devices(&home)?
            .into_iter()
            .find(|device| device.mac == mac)
        {
            white_range = device.white_range;
            if let Some(signing) = find_signing_key(&home) {
                key = parse_key(signing)?;
            }
        }
    }
    Ok(Some(LightConfig {
        ip,
        mac,
        key,
        white_range,
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
            .with_min_inner_size([520.0, 580.0])
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
