//! Estado compartido de WiZ Tray: preferencias, conexiones y catálogo de escenas.
//!
//! Vive en la biblioteca para que la interfaz (web o nativa) no tenga que
//! reimplementar la persistencia, el formato de `settings.json` ni la
//! traducción entre las preferencias guardadas y las conexiones reales.

use std::{
    env, fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Action, CloudConfig, CloudOAuthConfig, ExportDevice, LightConfig, PREFERRED_SCENE, Pilot,
    find_signing_key, normalize_mac, parse_cloud_id, parse_export_devices, parse_key,
    parse_white_range, validate_cloud_config,
};

/// Catálogo de escenas de WiZ: identificador y nombre.
pub const SCENES: &[(u16, &str)] = &[
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

/// Escenas que aparecen en la lista corta de la pantalla principal.
pub const FAVORITE_SCENES: &[u16] = &[PREFERRED_SCENE, 11, 13, 15, 16, 14];

/// De dónde llega el control del foco.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Local,
    Cloud,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cloud => "cloud",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Casa por Wi-Fi",
            Self::Cloud => "WiZ Cloud remoto",
        }
    }

    pub fn from_value(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(Value::as_str).unwrap_or("local") {
            "local" => Ok(Self::Local),
            "cloud" => Ok(Self::Cloud),
            _ => Err("La conexión guardada debe ser local o WiZ Cloud.".to_string()),
        }
    }
}

/// Acción que ejecuta la rutina diaria.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutineAction {
    Off,
    On,
    Daylight,
}

impl RoutineAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Daylight => "daylight",
        }
    }

    pub fn from_str(value: Option<&str>) -> Result<Self, String> {
        match value {
            Some("off") => Ok(Self::Off),
            Some("on") => Ok(Self::On),
            Some("daylight") => Ok(Self::Daylight),
            _ => Err("La acción guardada de la rutina diaria no es válida.".to_string()),
        }
    }

    pub fn action(self) -> Action {
        match self {
            Self::Off => Action::Off,
            Self::On => Action::On,
            Self::Daylight => Action::Daylight,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Apagar",
            Self::On => "Encender",
            Self::Daylight => "Encender con Luz de día",
        }
    }
}

/// Rutina diaria programada.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyRoutine {
    pub enabled: bool,
    pub minute_of_day: u16,
    pub action: RoutineAction,
    pub last_run_day: Option<u32>,
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

/// Preferencias guardadas en `%LOCALAPPDATA%\WiZ Tray\settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub backend: Backend,
    pub device_name: String,
    pub local_export: String,
    pub local_ip: String,
    pub local_mac: String,
    pub cloud_device_id: String,
    pub cloud_home_id: String,
    pub cloud_mac: String,
    pub cloud_white_range: Option<(u16, u16)>,
    pub daily_routine: DailyRoutine,
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

/// Conexiones ya configuradas para cada modo.
#[derive(Clone, Default)]
pub struct Connections {
    pub local: Option<LightConfig>,
    pub cloud: Option<CloudConfig>,
}

impl Connections {
    pub fn has(&self, backend: Backend) -> bool {
        match backend {
            Backend::Local => self.local.is_some(),
            Backend::Cloud => self.cloud.is_some(),
        }
    }

    pub fn white_range(&self, backend: Backend) -> Option<(u16, u16)> {
        match backend {
            Backend::Local => self.local.as_ref()?.white_range,
            Backend::Cloud => self.cloud.as_ref()?.white_range,
        }
    }

    pub fn perform(&self, backend: Backend, action: &Action) -> Result<Pilot, String> {
        match backend {
            Backend::Local => self
                .local
                .as_ref()
                .ok_or_else(|| "Configura primero la conexión local.".to_string())
                .and_then(|config| crate::perform(config, action)),
            Backend::Cloud => self
                .cloud
                .as_ref()
                .ok_or_else(|| "Configura primero WiZ Cloud.".to_string())
                .and_then(|config| crate::perform_cloud(config, action)),
        }
    }
}

/// Ruta del archivo de preferencias.
pub fn app_settings_path() -> Result<PathBuf, String> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("WiZ Tray").join("settings.json"))
        .ok_or_else(|| "Windows no expuso la carpeta local para guardar WiZ Tray.".to_string())
}

/// Formato en disco de las preferencias.
pub fn preferences_json(preferences: &Preferences) -> Value {
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

/// Lee la rutina diaria del JSON guardado.
pub fn daily_routine_from_json(value: Option<&Value>) -> Result<DailyRoutine, String> {
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

/// Lee las preferencias del JSON guardado.
pub fn preferences_from_json(value: &Value) -> Result<Preferences, String> {
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

/// Ruta del `wiz-tray.json` antiguo, junto al ejecutable.
pub fn legacy_settings_path() -> Option<PathBuf> {
    env::current_exe()
        .ok()?
        .parent()
        .map(|path| path.join("wiz-tray.json"))
}

/// Carga las preferencias guardadas, migrando el formato antiguo si hace falta.
pub fn load_preferences() -> Result<Preferences, String> {
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

/// Borra del archivo antiguo las credenciales que ahora guarda DPAPI.
pub fn redact_legacy_credentials(path: &Path, mut settings: Value) -> Result<(), String> {
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

/// Guarda las preferencias sin secretos.
pub fn save_preferences(preferences: &Preferences) -> Result<(), String> {
    let path = app_settings_path()?;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "No pude preparar la carpeta de configuración.".to_string())?,
    )
    .map_err(|_| "No pude preparar la carpeta de configuración de WiZ Tray.".to_string())?;
    write_json(&path, &preferences_json(preferences))
}

/// Escribe JSON de forma atómica.
pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| "No pude preparar la configuración de WiZ Tray.".to_string())?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes)
        .map_err(|_| "No pude guardar la configuración de WiZ Tray.".to_string())?;
    replace_file(&temp, path)
        .map_err(|_| "No pude finalizar la configuración de WiZ Tray.".to_string())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

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

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

/// Construye las conexiones reales a partir de las preferencias guardadas.
pub fn build_connections(
    preferences: &Preferences,
    oauth: Option<CloudOAuthConfig>,
) -> Result<Connections, String> {
    Ok(Connections {
        local: build_local_config(preferences)?,
        cloud: build_cloud_config(preferences, oauth)?,
    })
}

/// Configura el control local a partir de las preferencias.
pub fn build_local_config(preferences: &Preferences) -> Result<Option<LightConfig>, String> {
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

/// Configura el control por WiZ Cloud a partir de las preferencias.
pub fn build_cloud_config(
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

/// Interpreta el rango de temperatura escrito por la persona.
pub fn parse_optional_range(minimum: &str, maximum: &str) -> Result<Option<(u16, u16)>, String> {
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

/// Lee los focos de un export de WiZ.
pub fn load_export_devices(path: &str) -> Result<Vec<ExportDevice>, String> {
    let bytes =
        fs::read(path).map_err(|_| "No pude leer el export WiZ seleccionado.".to_string())?;
    let home: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "El export WiZ seleccionado no es JSON válido.".to_string())?;
    parse_export_devices(&home)
}

/// Busca el export de WiZ en las rutas habituales.
pub fn locate_export() -> Result<PathBuf, String> {
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

/// Nombre de la escena activa.
pub fn scene_name(scene_id: Option<u16>) -> &'static str {
    scene_id
        .and_then(|id| {
            SCENES
                .iter()
                .find(|(scene, _)| *scene == id)
                .map(|(_, name)| *name)
        })
        .unwrap_or("Modo actual")
}

/// Escena de la lista corta o del catálogo completo.
pub fn scene_at(index: usize, show_all: bool) -> Option<(u16, &'static str)> {
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

/// Color aproximado de cada escena, como RGB.
pub fn scene_rgb(scene_id: u16) -> (u8, u8, u8) {
    match scene_id {
        PREFERRED_SCENE => (232, 172, 52),
        11 => (232, 156, 74),
        13 => (116, 166, 204),
        15 => (82, 156, 130),
        16 => (140, 118, 198),
        14 => (74, 92, 168),
        1 => (52, 138, 189),
        2 => (224, 96, 132),
        3 => (240, 133, 69),
        4 => (198, 84, 206),
        5 => (214, 105, 48),
        6 => (196, 148, 96),
        7 => (72, 150, 91),
        8 => (210, 156, 196),
        9 => (243, 188, 92),
        10 => (70, 82, 140),
        17 => (96, 165, 196),
        18 => (104, 118, 168),
        19 => (98, 168, 108),
        20 => (150, 196, 90),
        21 => (240, 178, 56),
        22 => (196, 120, 60),
        23 => (42, 108, 160),
        24 => (48, 140, 96),
        25 => (120, 200, 140),
        26 => (170, 70, 200),
        27 => (200, 60, 70),
        28 => (240, 130, 40),
        _ => (148, 148, 160),
    }
}

/// Escenas que cambian de color con el tiempo, para la vista previa.
pub fn scene_is_animated(scene_id: u16) -> bool {
    matches!(scene_id, 1..=10 | 17..=28)
}

/// Mezcla dos colores RGB.
pub fn mix_rgb(from: (u8, u8, u8), to: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 * (1.0 - amount) + b as f32 * amount).round() as u8;
    (
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

/// Color que muestra el foquito para el estado real del foco.
pub fn preview_rgb(pilot: Option<Pilot>) -> (u8, u8, u8) {
    let Some(pilot) = pilot.filter(|pilot| pilot.state) else {
        return (58, 58, 66);
    };
    if let Some(scene) = pilot.scene_id.filter(|scene| *scene != 0) {
        return scene_rgb(scene);
    }
    if let Some((red, green, blue)) = pilot.color {
        return (red, green, blue);
    }
    let temperature = f32::from(pilot.temp.unwrap_or(4000));
    mix_rgb(
        (255, 180, 95),
        (190, 220, 255),
        (temperature - 2200.0) / 4300.0,
    )
}

/// Día local (AAAAMMDD) y minuto del día, para la rutina diaria.
#[cfg(windows)]
pub fn local_day_and_minute() -> (u32, u16) {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut now = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut now) };
    let day = u32::from(now.wYear) * 10_000 + u32::from(now.wMonth) * 100 + u32::from(now.wDay);
    let minute = u16::from(now.wHour) * 60 + u16::from(now.wMinute);
    (day, minute)
}

#[cfg(not(windows))]
pub fn local_day_and_minute() -> (u32, u16) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let day = (now.as_secs() / 86_400) as u32 + 19_700_000;
    let minute = ((now.as_secs() % 86_400) / 60) as u16;
    (day, minute)
}

/// Acción que la rutina diaria debe ejecutar ahora, si toca.
pub fn routine_action_due(routine: &DailyRoutine, today: u32, minute: u16) -> Option<Action> {
    (routine.enabled && routine.minute_of_day == minute && routine.last_run_day != Some(today))
        .then(|| routine.action.action())
}
