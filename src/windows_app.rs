use std::{collections::HashMap, env, fs, path::PathBuf, thread};

use serde_json::Value;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};

use wiz_tray::{
    Action, CloudConfig, LightConfig, PREFERRED_SCENE, Pilot, find_signing_key, normalize_mac,
    parse_cloud_id, parse_cloud_oauth, parse_key, parse_white_range, perform, perform_cloud,
    temperature_options, validate_cloud_config,
};

const SCENES: &[(u16, &str)] = &[
    (PREFERRED_SCENE, "Luz de día"),
    (11, "Blanco cálido"),
    (13, "Blanco frío"),
    (15, "Enfoque"),
    (16, "Relajarse"),
    (14, "Luz nocturna"),
];

enum UiEvent {
    Menu(MenuEvent),
    Finished(Result<Pilot, String>),
}

struct App {
    config: ConnectionConfig,
    proxy: EventLoopProxy<UiEvent>,
    tray: Option<TrayIcon>,
    status: Option<MenuItem>,
    actions: HashMap<MenuId, Action>,
    command_running: bool,
}

#[derive(Clone)]
enum ConnectionConfig {
    Local(LightConfig),
    Cloud(CloudConfig),
}

impl ConnectionConfig {
    fn white_range(&self) -> Option<(u16, u16)> {
        match self {
            Self::Local(config) => config.white_range,
            Self::Cloud(config) => config.white_range,
        }
    }

    fn perform(&self, action: &Action) -> Result<Pilot, String> {
        match self {
            Self::Local(config) => perform(config, action),
            Self::Cloud(config) => perform_cloud(config, action),
        }
    }
}

impl App {
    fn new(config: ConnectionConfig, proxy: EventLoopProxy<UiEvent>) -> Self {
        Self {
            config,
            proxy,
            tray: None,
            status: None,
            actions: HashMap::new(),
            command_running: false,
        }
    }

    fn install_tray(&mut self) -> Result<(), String> {
        let menu = Menu::new();
        let status = MenuItem::new("WiZ · conectando…", false, None);
        menu.append(&status).map_err(|error| error.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        if matches!(&self.config, ConnectionConfig::Cloud(_)) {
            append_action(
                &menu,
                "Conectar WiZ Cloud con Google",
                Action::Connect,
                &mut self.actions,
            )?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|error| error.to_string())?;
        }

        append_action(
            &menu,
            "Encender · Luz de día",
            Action::Daylight,
            &mut self.actions,
        )?;
        append_action(&menu, "Encender", Action::On, &mut self.actions)?;
        append_action(&menu, "Apagar", Action::Off, &mut self.actions)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;

        for (scene_id, name) in SCENES {
            append_action(
                &menu,
                &format!("Modo · {name}"),
                Action::Scene(*scene_id),
                &mut self.actions,
            )?;
        }

        if let Some(range) = self.config.white_range() {
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|error| error.to_string())?;
            for temperature in temperature_options(range) {
                append_action(
                    &menu,
                    &format!("Temperatura · {temperature} K"),
                    Action::Temperature(temperature),
                    &mut self.actions,
                )?;
            }
        }

        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        append_action(
            &menu,
            "Actualizar estado",
            Action::Refresh,
            &mut self.actions,
        )?;
        append_action(&menu, "Salir", Action::Exit, &mut self.actions)?;

        self.tray = Some(
            TrayIconBuilder::new()
                .with_icon(make_icon(false)?)
                .with_tooltip("WiZ · conectando…")
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(true)
                .build()
                .map_err(|error| error.to_string())?,
        );
        self.status = Some(status);
        Ok(())
    }

    fn set_status(&self, text: &str) {
        if let Some(status) = &self.status {
            let _ = status.set_text(text);
        }
        if let Some(tray) = &self.tray {
            let _ = tray.set_tooltip(Some(text));
        }
    }

    fn start_action(&mut self, action: Action) {
        if self.command_running {
            self.set_status("WiZ · espera a que termine el comando");
            return;
        }

        self.command_running = true;
        self.set_status("WiZ · enviando y verificando…");
        let config = self.config.clone();
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = config.perform(&action);
            let _ = proxy.send_event(UiEvent::Finished(result));
        });
    }

    fn finish_action(&mut self, result: Result<Pilot, String>) {
        self.command_running = false;
        match result {
            Ok(pilot) => {
                let status = if pilot.state {
                    let scene = scene_name(pilot.scene_id);
                    let mode = if scene != "modo actual" {
                        scene.to_string()
                    } else {
                        pilot
                            .temp
                            .map(|temperature| format!("{temperature} K"))
                            .unwrap_or_else(|| scene.to_string())
                    };
                    format!("Encendida · {mode}")
                } else {
                    "Apagada".to_string()
                };
                self.set_status(&format!("WiZ · {status}"));
                if let Some(tray) = &self.tray {
                    if let Ok(icon) = make_icon(pilot.state) {
                        let _ = tray.set_icon(Some(icon));
                    }
                }
            }
            Err(error) => self.set_status(&format!("WiZ · {error}")),
        }
    }
}

impl ApplicationHandler<UiEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.tray.is_some() {
            return;
        }
        if let Err(error) = self.install_tray() {
            show_error(&format!("No pude iniciar WiZ Tray:\n\n{error}"));
            event_loop.exit();
            return;
        }
        self.start_action(Action::Refresh);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiEvent) {
        match event {
            UiEvent::Menu(event) => {
                let Some(action) = self.actions.get(&event.id).cloned() else {
                    return;
                };
                match action {
                    Action::Exit => event_loop.exit(),
                    action => self.start_action(action),
                }
            }
            UiEvent::Finished(result) => self.finish_action(result),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
    }
}

fn append_action(
    menu: &Menu,
    label: &str,
    action: Action,
    actions: &mut HashMap<MenuId, Action>,
) -> Result<(), String> {
    let item = MenuItem::new(label, true, None);
    menu.append(&item).map_err(|error| error.to_string())?;
    actions.insert(item.id().clone(), action);
    Ok(())
}

fn make_icon(on: bool) -> Result<Icon, String> {
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
                rgba[pixel..pixel + 4].copy_from_slice(&[26, 43, 57, 255]);
            }
            if head || neck || base {
                rgba[pixel..pixel + 4].copy_from_slice(&bulb);
            }
            if (23..=23).contains(&y) && (13..=18).contains(&x) {
                rgba[pixel..pixel + 4].copy_from_slice(&[245, 239, 220, 255]);
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).map_err(|error| error.to_string())
}

fn scene_name(scene_id: Option<u16>) -> &'static str {
    scene_id
        .and_then(|id| {
            SCENES
                .iter()
                .find(|(scene, _)| *scene == id)
                .map(|(_, name)| *name)
        })
        .unwrap_or("modo actual")
}

fn load_config() -> Result<ConnectionConfig, String> {
    let settings_path = settings_path()
        .ok_or_else(|| "No pude localizar la configuración junto a la aplicación.".to_string())?;
    let bytes =
        fs::read(settings_path).map_err(|error| format!("No pude leer wiz-tray.json: {error}"))?;
    let settings: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("wiz-tray.json no es JSON válido: {error}"))?;
    let backend = match settings.get("backend") {
        None => "local",
        Some(Value::String(backend)) => backend,
        Some(_) => return Err("wiz-tray.json: backend debe ser local o cloud.".to_string()),
    };
    if backend == "cloud" {
        let device_id = parse_cloud_id(settings.get("device_id"), "device_id")?;
        let home_id = parse_cloud_id(settings.get("home_id"), "home_id")?;
        let mac_address = settings
            .get("mac_address")
            .and_then(Value::as_str)
            .ok_or_else(|| "wiz-tray.json no contiene mac_address para WiZ Cloud.".to_string())?;
        let white_range = settings
            .get("white_range")
            .map(|range| parse_white_range(&serde_json::json!({"white_range": range})))
            .transpose()?;
        let oauth = parse_cloud_oauth(&settings)?;
        let config = CloudConfig {
            device_id,
            home_id,
            mac_address: mac_address.to_string(),
            white_range,
            oauth,
        };
        validate_cloud_config(&config)?;
        return Ok(ConnectionConfig::Cloud(config));
    }
    if backend != "local" {
        return Err("wiz-tray.json: backend debe ser local o cloud.".to_string());
    }

    let export_path = locate_export()?;
    let bytes =
        fs::read(&export_path).map_err(|error| format!("No pude leer el export WiZ: {error}"))?;
    let home: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("El export WiZ no es JSON válido: {error}"))?;
    let device = home
        .get("devices")
        .and_then(Value::as_array)
        .and_then(|devices| devices.first())
        .ok_or_else(|| "El export WiZ no contiene un foco.".to_string())?;
    let mac = device
        .get("mac_address")
        .and_then(Value::as_str)
        .map(normalize_mac)
        .filter(|mac| mac.len() == 12)
        .ok_or_else(|| "El export WiZ no contiene la identidad del foco.".to_string())?;
    let key_text = find_signing_key(&home)
        .ok_or_else(|| "El export WiZ no contiene la clave de control local.".to_string())?;
    let key = parse_key(key_text)?;

    let ip = settings
        .get("device_ip")
        .and_then(Value::as_str)
        .ok_or_else(|| "wiz-tray.json no contiene device_ip.".to_string())?
        .parse()
        .map_err(|error| format!("La IP configurada no es válida: {error}"))?;

    let white_range = device
        .get("traits")
        .filter(|traits| traits.get("is_tunable_white").and_then(Value::as_bool) == Some(true))
        .map(parse_white_range)
        .transpose()?;

    Ok(ConnectionConfig::Local(LightConfig {
        ip,
        mac,
        key,
        white_range,
    }))
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
    let candidates = [
        exe_dir.join("wiz-home.json"),
        exe_dir.join("..").join("wiz-home.json"),
        exe_dir.join("..").join("work").join("wiz-home.json"),
        env::current_dir().unwrap_or_default().join("wiz-home.json"),
        env::current_dir()
            .unwrap_or_default()
            .join("work")
            .join("wiz-home.json"),
    ];
    candidates
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "No encuentro wiz-home.json. Conserva el export junto al ejecutable o define WIZ_HOME_JSON.".to_string()
        })
}

fn settings_path() -> Option<PathBuf> {
    env::current_exe()
        .ok()?
        .parent()
        .map(|path| path.join("wiz-tray.json"))
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
    let config = load_config()?;
    let event_loop = EventLoop::<UiEvent>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UiEvent::Menu(event));
    }));
    let mut app = App::new(config, event_loop.create_proxy());
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())
}
