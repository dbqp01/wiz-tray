//! Estado de la aplicación y comandos que expone la interfaz web.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use wiz_tray::{
    clear_cloud_session, discover_lights, has_cloud_session, load_cloud_credentials,
    save_cloud_credentials,
    state::{self, Backend, Connections, DailyRoutine, Preferences, RoutineAction},
    Action, CloudOAuthConfig, DiscoveredLight, Pilot,
};

/// Estado compartido entre la interfaz, la bandeja y el hilo de sondeo.
pub type Shared = Arc<Mutex<AppState>>;

/// Todo lo que la app guarda entre sesiones y durante la ejecución.
pub struct AppState {
    pub preferences: Preferences,
    pub draft: Preferences,
    pub connections: Connections,
    pub cloud_oauth: Option<CloudOAuthConfig>,
    pub pilot: Option<Pilot>,
    pub brightness: u8,
    pub temperature: u16,
    pub color: [u8; 3],
    pub timer_due: Option<Instant>,
    pub status: String,
    pub error: Option<String>,
    pub busy: bool,
    pub show_all_scenes: bool,
}

impl AppState {
    fn new() -> Self {
        let preferences = state::load_preferences().unwrap_or_default();
        let cloud_oauth = load_cloud_credentials().ok().flatten();
        let connections =
            state::build_connections(&preferences, cloud_oauth.clone()).unwrap_or_default();
        Self {
            draft: preferences.clone(),
            preferences,
            connections,
            cloud_oauth,
            pilot: None,
            brightness: 100,
            temperature: 4000,
            color: [255, 226, 176],
            timer_due: None,
            status: "Listo".to_string(),
            error: None,
            busy: false,
            show_all_scenes: true,
        }
    }
}

/// Crea el estado compartido inicializándolo desde disco.
pub fn shared() -> Shared {
    Arc::new(Mutex::new(AppState::new()))
}

/// Una escena de WiZ con su color.
#[derive(Clone, Serialize)]
pub struct SceneInfo {
    pub id: u16,
    pub name: String,
    pub color: [u8; 3],
    pub animated: bool,
}

/// Colores directos que se pueden aplicar sin pasar por una escena.
const COLOR_PRESETS: &[u16] = &[1, 2, 3, 4, 5, 7, 8, 27];

/// Todo lo que la interfaz necesita dibujar en una pasada.
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub backend: String,
    pub backend_label: String,
    pub device_name: String,
    pub connected: bool,
    pub has_local: bool,
    pub has_cloud: bool,
    pub cloud_session: bool,
    pub cloud_ready: bool,
    pub pilot: Option<Pilot>,
    pub brightness: u8,
    pub temperature: u16,
    pub color: [u8; 3],
    pub white_range: Option<(u16, u16)>,
    pub favorites: Vec<SceneInfo>,
    pub scenes: Vec<SceneInfo>,
    pub colors: Vec<[u8; 3]>,
    pub timer_left: Option<u64>,
    pub status: String,
    pub error: Option<String>,
    pub busy: bool,
    pub routine: DailyRoutine,
    pub local_ip: String,
    pub local_mac: String,
    pub local_export: String,
    pub cloud_device_id: String,
    pub cloud_home_id: String,
    pub cloud_mac: String,
    pub cloud_min: String,
    pub cloud_max: String,
}

fn scene_info(id: u16, name: &str) -> SceneInfo {
    SceneInfo {
        id,
        name: name.to_string(),
        color: state::scene_rgb(id).into(),
        animated: state::scene_is_animated(id),
    }
}

fn catalog(show_all: bool) -> Vec<SceneInfo> {
    let total = if show_all {
        state::SCENES.len()
    } else {
        state::FAVORITE_SCENES.len()
    };
    (0..total)
        .filter_map(|index| state::scene_at(index, show_all))
        .map(|(id, name)| scene_info(id, name))
        .collect()
}

fn snapshot_of(state: &AppState) -> Snapshot {
    let backend = state.preferences.backend;
    let timer_left = state
        .timer_due
        .map(|due| due.saturating_duration_since(Instant::now()).as_secs());
    Snapshot {
        backend: backend.as_str().to_string(),
        backend_label: backend.label().to_string(),
        device_name: state.preferences.device_name.clone(),
        connected: state.connections.has(backend),
        has_local: state.connections.has(Backend::Local),
        has_cloud: state.connections.has(Backend::Cloud),
        cloud_session: has_cloud_session(),
        cloud_ready: state.cloud_oauth.is_some(),
        pilot: state.pilot,
        brightness: state.brightness,
        temperature: state.temperature,
        color: state.color,
        white_range: state.connections.white_range(backend),
        favorites: catalog(false),
        scenes: catalog(state.show_all_scenes),
        colors: COLOR_PRESETS
            .iter()
            .map(|id| state::scene_rgb(*id).into())
            .collect(),
        timer_left,
        status: state.status.clone(),
        error: state.error.clone(),
        busy: state.busy,
        routine: state.preferences.daily_routine.clone(),
        local_ip: state.draft.local_ip.clone(),
        local_mac: state.draft.local_mac.clone(),
        local_export: state.draft.local_export.clone(),
        cloud_device_id: state.draft.cloud_device_id.clone(),
        cloud_home_id: state.draft.cloud_home_id.clone(),
        cloud_mac: state.draft.cloud_mac.clone(),
        cloud_min: state
            .draft
            .cloud_white_range
            .map(|range| range.0.to_string())
            .unwrap_or_default(),
        cloud_max: state
            .draft
            .cloud_white_range
            .map(|range| range.1.to_string())
            .unwrap_or_default(),
    }
}

/// Aplica la lectura del foco al estado y deja la interfaz al día.
pub fn apply_pilot(state: &mut AppState, pilot: Pilot) {
    if let Some(dimming) = pilot.dimming {
        state.brightness = dimming;
    }
    if let Some(temp) = pilot.temp {
        state.temperature = temp;
    }
    if let Some((red, green, blue)) = pilot.color {
        state.color = [red, green, blue];
    }
    state.pilot = Some(pilot);
}

fn action_from(kind: &str, value: Option<&Value>) -> Result<Action, String> {
    match kind {
        "on" => Ok(Action::On),
        "off" => Ok(Action::Off),
        "daylight" => Ok(Action::Daylight),
        "connect" => Ok(Action::Connect),
        "refresh" => Ok(Action::Refresh),
        "scene" => Ok(Action::Scene(number(value, "Escena no válida.")? as u16)),
        "temperature" => Ok(Action::Temperature(
            number(value, "Temperatura no válida.")? as u16,
        )),
        "brightness" => Ok(Action::Brightness(number(value, "Brillo no válido.")? as u8)),
        "color" => {
            let rgb = value
                .and_then(Value::as_array)
                .ok_or_else(|| "Color no válido.".to_string())?;
            if rgb.len() != 3 {
                return Err("Color no válido.".to_string());
            }
            Ok(Action::Color(
                rgb[0].as_u64().unwrap_or(0) as u8,
                rgb[1].as_u64().unwrap_or(0) as u8,
                rgb[2].as_u64().unwrap_or(0) as u8,
            ))
        }
        other => Err(format!("Acción desconocida: {other}")),
    }
}

fn number(value: Option<&Value>, message: &str) -> Result<u64, String> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| message.to_string())
}

/// Estado actual para la interfaz.
#[tauri::command]
pub fn snapshot(state: State<'_, Shared>) -> Snapshot {
    snapshot_of(&state.lock().expect("estado de WiZ Tray"))
}

/// Ejecuta una acción contra el foco y devuelve el estado resultante.
#[tauri::command]
pub async fn perform(
    kind: String,
    value: Option<Value>,
    state: State<'_, Shared>,
) -> Result<Snapshot, String> {
    let action = action_from(&kind, value.as_ref())?;
    let (backend, connections) = {
        let guard = state.lock().expect("estado de WiZ Tray");
        (guard.preferences.backend, guard.connections.clone())
    };
    if !connections.has(backend) {
        return Err("Configura primero la conexión.".to_string());
    }
    {
        let mut guard = state.lock().expect("estado de WiZ Tray");
        match &action {
            Action::Brightness(brightness) => guard.brightness = *brightness,
            Action::Temperature(temperature) => guard.temperature = *temperature,
            Action::Color(red, green, blue) => guard.color = [*red, *green, *blue],
            _ => {}
        }
        guard.busy = true;
        guard.error = None;
    }
    let description = describe(&action);
    let result =
        tauri::async_runtime::spawn_blocking(move || connections.perform(backend, &action))
            .await
            .map_err(|error| format!("No pude ejecutar la acción: {error}"))?;
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.busy = false;
    match result {
        Ok(pilot) => {
            apply_pilot(&mut guard, pilot);
            guard.status = description;
        }
        Err(error) => {
            guard.error = Some(error);
        }
    }
    Ok(snapshot_of(&guard))
}

fn describe(action: &Action) -> String {
    match action {
        Action::On => "Luz encendida".to_string(),
        Action::Off => "Luz apagada".to_string(),
        Action::Daylight => "Luz de día aplicada".to_string(),
        Action::Connect => "Sesión de WiZ Cloud iniciada".to_string(),
        Action::Scene(id) => format!("Escena {} aplicada", state::scene_name(Some(*id))),
        Action::Temperature(value) => format!("Temperatura en {value} K"),
        Action::Brightness(value) => format!("Brillo al {value} %"),
        Action::Color(_, _, _) => "Color aplicado".to_string(),
        _ => "Estado actualizado".to_string(),
    }
}

/// Cambia entre control local y WiZ Cloud.
#[tauri::command]
pub fn set_backend(backend: String, state: State<'_, Shared>) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.preferences.backend = Backend::from_value(Some(&Value::String(backend)))?;
    state::save_preferences(&guard.preferences)?;
    guard.draft.backend = guard.preferences.backend;
    guard.status = format!("Conexión: {}", guard.preferences.backend.label());
    Ok(snapshot_of(&guard))
}

/// Muestra u oculta todas las escenas del catálogo.
#[tauri::command]
pub fn set_show_all_scenes(show_all: bool, state: State<'_, Shared>) -> Snapshot {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.show_all_scenes = show_all;
    snapshot_of(&guard)
}

/// Busca focos de WiZ en la red local.
#[tauri::command]
pub async fn discover() -> Result<Vec<DiscoveredLight>, String> {
    tauri::async_runtime::spawn_blocking(discover_lights)
        .await
        .map_err(|error| format!("No pude buscar focos: {error}"))?
}

/// Guarda la conexión local elegida.
#[tauri::command]
pub fn save_local_setup(
    export_path: String,
    ip: String,
    mac: String,
    device_name: String,
    state: State<'_, Shared>,
) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.draft.local_export = export_path;
    guard.draft.local_ip = ip;
    guard.draft.local_mac = mac;
    if !device_name.trim().is_empty() {
        guard.draft.device_name = device_name.trim().to_string();
    }
    guard.preferences = guard.draft.clone();
    state::save_preferences(&guard.preferences)?;
    guard.connections = state::build_connections(&guard.preferences, guard.cloud_oauth.clone())?;
    guard.status = "Conexión local guardada".to_string();
    Ok(snapshot_of(&guard))
}

/// Guarda la conexión de WiZ Cloud elegida.
#[tauri::command]
pub fn save_cloud_setup(
    device_id: String,
    home_id: String,
    mac: String,
    minimum: String,
    maximum: String,
    state: State<'_, Shared>,
) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.draft.cloud_device_id = device_id;
    guard.draft.cloud_home_id = home_id;
    guard.draft.cloud_mac = mac;
    guard.draft.cloud_white_range = state::parse_optional_range(&minimum, &maximum)?;
    guard.preferences = guard.draft.clone();
    state::save_preferences(&guard.preferences)?;
    guard.connections = state::build_connections(&guard.preferences, guard.cloud_oauth.clone())?;
    guard.status = "Conexión Cloud guardada".to_string();
    Ok(snapshot_of(&guard))
}

/// Guarda las credenciales OAuth de la app WiZ, cifradas con DPAPI.
#[tauri::command]
pub fn save_cloud_oauth(
    google_client_id: String,
    google_client_secret: String,
    wiz_client_id: String,
    wiz_client_secret: String,
    state: State<'_, Shared>,
) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    let oauth = CloudOAuthConfig {
        google_client_id: google_client_id.trim().to_string(),
        google_client_secret: Some(google_client_secret.trim().to_string())
            .filter(|value| !value.is_empty()),
        wiz_client_id: wiz_client_id.trim().to_string(),
        wiz_client_secret: wiz_client_secret.trim().to_string(),
    };
    save_cloud_credentials(&oauth)?;
    guard.cloud_oauth = Some(oauth);
    guard.connections = state::build_connections(&guard.preferences, guard.cloud_oauth.clone())?;
    guard.status = "Credenciales de WiZ Cloud guardadas".to_string();
    Ok(snapshot_of(&guard))
}

/// Borra la sesión de WiZ Cloud de este perfil de Windows.
#[tauri::command]
pub fn clear_cloud_session_command(state: State<'_, Shared>) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    clear_cloud_session()?;
    guard.status = "Sesión de WiZ Cloud cerrada".to_string();
    Ok(snapshot_of(&guard))
}

/// Guarda la rutina diaria.
#[tauri::command]
pub fn set_routine(
    enabled: bool,
    minute_of_day: u16,
    action: String,
    state: State<'_, Shared>,
) -> Result<Snapshot, String> {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    let routine = DailyRoutine {
        enabled,
        minute_of_day,
        action: RoutineAction::from_str(Some(action.as_str()))?,
        last_run_day: guard.preferences.daily_routine.last_run_day,
    };
    guard.draft.daily_routine = routine.clone();
    guard.preferences.daily_routine = routine;
    state::save_preferences(&guard.preferences)?;
    guard.status = "Rutina diaria guardada".to_string();
    Ok(snapshot_of(&guard))
}

/// Programa el apagado automático.
#[tauri::command]
pub fn start_timer(minutes: u16, state: State<'_, Shared>) -> Snapshot {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.timer_due = Some(Instant::now() + Duration::from_secs(u64::from(minutes) * 60));
    guard.status = format!("Apagado programado en {minutes} minutos");
    snapshot_of(&guard)
}

/// Cancela el apagado automático.
#[tauri::command]
pub fn cancel_timer(state: State<'_, Shared>) -> Snapshot {
    let mut guard = state.lock().expect("estado de WiZ Tray");
    guard.timer_due = None;
    guard.status = "Rutina cancelada".to_string();
    snapshot_of(&guard)
}

/// Aplica una escena de la lista.
#[tauri::command]
pub async fn apply_scene(id: u16, state: State<'_, Shared>) -> Result<Snapshot, String> {
    perform("scene".to_string(), Some(Value::from(id)), state).await
}

/// Refresca la lectura del foco.
#[tauri::command]
pub async fn refresh(state: State<'_, Shared>) -> Result<Snapshot, String> {
    perform("refresh".to_string(), None, state).await
}

/// Publica un estado nuevo hacia la interfaz.
pub fn publish(app: &AppHandle, state: &Shared) {
    let snapshot = snapshot_of(&state.lock().expect("estado de WiZ Tray"));
    let _ = app.emit("state", snapshot);
}

/// Abre el selector de archivos para elegir el export de la app WiZ.
#[tauri::command]
pub async fn pick_export() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Export de WiZ", &["json"])
            .pick_file()
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .map_err(|error| format!("No pude abrir el selector de archivos: {error}"))
}

/// Lee los focos que contiene un export de la app WiZ.
#[tauri::command]
pub async fn load_export(path: String) -> Result<Vec<wiz_tray::ExportDevice>, String> {
    tauri::async_runtime::spawn_blocking(move || state::load_export_devices(&path))
        .await
        .map_err(|error| format!("No pude leer el export: {error}"))?
}
