use std::{
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

#[cfg(windows)]
mod cloud_auth;

const DEVICE_PORT: u16 = 38899;
const WIZ_APP_VERSION: &str = "1.38.6";
pub const PREFERRED_SCENE: u16 = 12;
pub const TEMPERATURE_PRESETS: &[u16] = &[2200, 2700, 3500, 4000, 5000, 6500];

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct LightConfig {
    pub ip: Ipv4Addr,
    pub mac: String,
    pub key: Vec<u8>,
    pub white_range: Option<(u16, u16)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDevice {
    pub name: String,
    pub mac: String,
    pub white_range: Option<(u16, u16)>,
    pub device_id: Option<u64>,
    pub home_id: Option<u64>,
}

#[derive(Clone)]
pub struct CloudConfig {
    pub device_id: u64,
    pub home_id: u64,
    pub mac_address: String,
    pub white_range: Option<(u16, u16)>,
    pub oauth: Option<CloudOAuthConfig>,
}

#[derive(Clone)]
pub struct CloudOAuthConfig {
    pub google_client_id: String,
    pub google_client_secret: Option<String>,
    pub wiz_client_id: String,
    pub wiz_client_secret: String,
}

#[derive(Clone, Copy)]
pub struct Pilot {
    pub state: bool,
    pub scene_id: Option<u16>,
    pub temp: Option<u16>,
}

#[derive(Clone)]
pub enum Action {
    Daylight,
    On,
    Off,
    Connect,
    Scene(u16),
    Temperature(u16),
    Refresh,
    Exit,
}

pub fn parse_white_range(traits: &Value) -> Result<(u16, u16), String> {
    traits
        .get("white_range")
        .and_then(Value::as_array)
        .filter(|range| range.len() == 2)
        .and_then(|range| {
            Some((
                u16::try_from(range[0].as_u64()?).ok()?,
                u16::try_from(range[1].as_u64()?).ok()?,
            ))
        })
        .filter(|(min, max)| 1000 <= *min && *min < *max && *max <= 12000)
        .ok_or_else(|| "El rango de temperatura del export WiZ no es válido.".to_string())
}

pub fn temperature_options((min, max): (u16, u16)) -> Vec<u16> {
    let mut temperatures = vec![min, max];
    temperatures.extend(
        TEMPERATURE_PRESETS
            .iter()
            .copied()
            .filter(|temperature| min <= *temperature && *temperature <= max),
    );
    temperatures.sort_unstable();
    temperatures.dedup();
    temperatures
}

pub fn find_signing_key(value: &Value) -> Option<&str> {
    match value {
        Value::Object(object) => object
            .get("udp_signing_key")
            .and_then(Value::as_str)
            .or_else(|| object.values().find_map(find_signing_key)),
        Value::Array(items) => items.iter().find_map(find_signing_key),
        _ => None,
    }
}

pub fn parse_export_devices(value: &Value) -> Result<Vec<ExportDevice>, String> {
    let devices = value
        .get("devices")
        .and_then(Value::as_array)
        .filter(|devices| !devices.is_empty())
        .ok_or_else(|| "El export WiZ no contiene focos.".to_string())?;

    devices
        .iter()
        .map(|device| {
            let mac = device
                .get("mac_address")
                .and_then(Value::as_str)
                .filter(|mac| {
                    mac.chars().all(|character| {
                        character.is_ascii_hexdigit() || matches!(character, ':' | '-')
                    })
                })
                .map(normalize_mac)
                .filter(|mac| mac.len() == 12)
                .ok_or_else(|| "El export WiZ contiene un foco sin MAC válida.".to_string())?;
            let name = ["name", "device_name", "alias"]
                .iter()
                .find_map(|field| device.get(field).and_then(Value::as_str))
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("WiZ · {}", &mac[8..]));
            let white_range = device
                .get("traits")
                .filter(|traits| {
                    traits.get("is_tunable_white").and_then(Value::as_bool) == Some(true)
                })
                .map(parse_white_range)
                .transpose()?;

            Ok(ExportDevice {
                name,
                mac,
                white_range,
                device_id: parse_cloud_id(device.get("device_id"), "device_id").ok(),
                home_id: parse_cloud_id(value.get("home_id"), "home_id").ok(),
            })
        })
        .collect()
}

pub fn parse_key(value: &str) -> Result<Vec<u8>, String> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("La clave local WiZ no tiene el formato esperado.".to_string());
    }
    (0..16)
        .map(|index| {
            u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
                .map_err(|_| "La clave local WiZ no es hexadecimal.".to_string())
        })
        .collect()
}

pub fn parse_cloud_id(value: Option<&Value>, field: &str) -> Result<u64, String> {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("WiZ Cloud requiere un {field} válido."))
}

pub fn parse_cloud_oauth(settings: &Value) -> Result<Option<CloudOAuthConfig>, String> {
    let google_client_id = optional_string_setting(settings, "google_oauth_client_id")?;
    let google_client_secret = optional_string_setting(settings, "google_oauth_client_secret")?;
    let wiz_client_id = optional_string_setting(settings, "wiz_oauth_client_id")?;
    let wiz_client_secret = optional_string_setting(settings, "wiz_oauth_client_secret")?;
    match (
        google_client_id,
        google_client_secret,
        wiz_client_id,
        wiz_client_secret,
    ) {
        (None, None, None, None) => Ok(None),
        (Some(google_client_id), google_client_secret, Some(wiz_client_id), Some(wiz_client_secret)) => {
            Ok(Some(CloudOAuthConfig {
                google_client_id: google_client_id.to_string(),
                google_client_secret: google_client_secret.map(str::to_string),
                wiz_client_id: wiz_client_id.to_string(),
                wiz_client_secret: wiz_client_secret.to_string(),
            }))
        }
        _ => Err("wiz-tray.json debe incluir juntos google_oauth_client_id, wiz_oauth_client_id y wiz_oauth_client_secret; google_oauth_client_secret es opcional.".to_string()),
    }
}

fn optional_string_setting<'a>(settings: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match settings.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("wiz-tray.json: {key} debe ser texto.")),
    }
}

pub fn normalize_mac(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn perform(config: &LightConfig, action: &Action) -> Result<Pilot, String> {
    if matches!(action, Action::Connect) {
        return Err("Conectar con Google solo está disponible para WiZ Cloud.".to_string());
    }
    if matches!(action, Action::Refresh) {
        return get_pilot(config);
    }
    if matches!(action, Action::Exit) {
        return Err("acción no válida".to_string());
    }

    get_pilot(config)?;
    let (fields, expected_state, expected_scene, expected_temp) =
        command_fields(action).ok_or_else(|| "acción WiZ no reconocida".to_string())?;
    let response = exchange(config, "setPilot", &fields)?;
    if response.get("success") == Some(&Value::Bool(false)) {
        return Err("WiZ rechazó el comando".to_string());
    }

    let mut last_error = "El foco no confirmó el cambio.".to_string();
    for _ in 0..5 {
        thread::sleep(Duration::from_millis(400));
        match get_pilot(config) {
            Ok(pilot)
                if pilot.state == expected_state
                    && expected_scene.is_none_or(|scene| pilot.scene_id == Some(scene))
                    && expected_temp.is_none_or(|temp| pilot.temp == Some(temp)) =>
            {
                return Ok(pilot);
            }
            Ok(_) => last_error = "El foco respondió, pero el estado no coincide.".to_string(),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

pub fn perform_cloud(config: &CloudConfig, action: &Action) -> Result<Pilot, String> {
    validate_cloud_config(config)?;
    #[cfg(windows)]
    {
        let is_connecting = matches!(action, Action::Connect);
        let token = if is_connecting {
            cloud_auth::connect(config)?
        } else {
            cloud_auth::access_token(config)?
        };
        let refresh = Action::Refresh;
        let action = if is_connecting { &refresh } else { action };
        perform_cloud_with_token(config, action, &token, "https://api.wiz.world")
    }
    #[cfg(not(windows))]
    {
        let _ = (config, action);
        Err("El inicio de sesión WiZ Cloud solo está disponible en Windows.".to_string())
    }
}

fn perform_cloud_with_token(
    config: &CloudConfig,
    action: &Action,
    token: &str,
    base_url: &str,
) -> Result<Pilot, String> {
    validate_cloud_config(config)?;
    if token.is_empty() {
        return Err("El token de WiZ Cloud está vacío.".to_string());
    }
    if matches!(action, Action::Connect | Action::Exit) {
        return Err("acción no válida".to_string());
    }

    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();
    let base_url = base_url.trim_end_matches('/');
    let home_url = format!("{base_url}/api/v2/home/{}", config.home_id);
    let pilot_url = format!("{base_url}/api/v2/light/{}/pilot", config.device_id);
    let get_pilot = || cloud_get_pilot(&agent, config, token, &home_url);
    if matches!(action, Action::Refresh) {
        return get_pilot();
    }

    let body = cloud_command_body(action)?;
    let response = cloud_request(&agent, config, token, &pilot_url, Some(&body))?;
    if response.get("success") == Some(&Value::Bool(false)) {
        return Err("WiZ Cloud rechazó el comando.".to_string());
    }

    let (expected_state, expected_scene, expected_temp) =
        cloud_expected_state(action).ok_or_else(|| "acción WiZ no reconocida".to_string())?;
    let mut last_error = "El foco no confirmó el cambio en WiZ Cloud.".to_string();
    // Cloud can briefly return stale state after a scene change, so read it back until it matches.
    for _ in 0..5 {
        thread::sleep(Duration::from_millis(400));
        match get_pilot() {
            Ok(pilot)
                if pilot.state == expected_state
                    && expected_scene.is_none_or(|scene| pilot.scene_id == Some(scene))
                    && expected_temp.is_none_or(|temperature| pilot.temp == Some(temperature)) =>
            {
                return Ok(pilot);
            }
            Ok(_) => last_error = "WiZ Cloud respondió, pero el estado no coincide.".to_string(),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

pub fn validate_cloud_config(config: &CloudConfig) -> Result<(), String> {
    if config.device_id == 0 {
        return Err("WiZ Cloud requiere un ID de foco válido.".to_string());
    }
    if config.home_id == 0 {
        return Err("WiZ Cloud requiere un ID de hogar válido.".to_string());
    }
    if config.mac_address.is_empty()
        || !config
            .mac_address
            .chars()
            .all(|character| character.is_ascii_hexdigit() || matches!(character, ':' | '-'))
        || normalize_mac(&config.mac_address).len() != 12
    {
        return Err("WiZ Cloud requiere una MAC de foco válida.".to_string());
    }
    if let Some(range) = config.white_range {
        if !(1000 <= range.0 && range.0 < range.1 && range.1 <= 12000) {
            return Err("El rango de temperatura WiZ no es válido.".to_string());
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn save_cloud_credentials(oauth: &CloudOAuthConfig) -> Result<(), String> {
    cloud_auth::save_oauth_config(oauth)
}

#[cfg(windows)]
pub fn load_cloud_credentials() -> Result<Option<CloudOAuthConfig>, String> {
    cloud_auth::load_oauth_config()
}

#[cfg(windows)]
pub fn has_cloud_session() -> bool {
    cloud_auth::has_saved_session()
}

#[cfg(windows)]
pub fn clear_cloud_session() -> Result<(), String> {
    cloud_auth::clear_session()
}

fn cloud_get_pilot(
    agent: &ureq::Agent,
    config: &CloudConfig,
    token: &str,
    url: &str,
) -> Result<Pilot, String> {
    let response = cloud_request(agent, config, token, url, None)?;
    if response.get("success") == Some(&Value::Bool(false)) {
        return Err("WiZ Cloud rechazó la lectura del hogar.".to_string());
    }
    let payload = response
        .get("data")
        .or_else(|| response.get("result"))
        .unwrap_or(&response);
    let light = payload
        .get("lights")
        .and_then(Value::as_array)
        .and_then(|lights| {
            lights.iter().find(|light| {
                light
                    .get("id")
                    .and_then(|id| id.as_u64().or_else(|| id.as_str()?.parse().ok()))
                    == Some(config.device_id)
            })
        })
        .ok_or_else(|| "WiZ Cloud no devolvió el foco configurado.".to_string())?;
    let pilot = light
        .get("current_state")
        .ok_or_else(|| "WiZ Cloud no devolvió el estado actual del foco.".to_string())?;
    let state = pilot
        .get("status")
        .and_then(Value::as_bool)
        .ok_or_else(|| "WiZ Cloud no devolvió un estado válido.".to_string())?;
    let temp = pilot
        .get("temperature")
        .and_then(Value::as_u64)
        .and_then(|temperature| u16::try_from(temperature).ok());
    // WiZ Cloud reports its daylight scene as 4200 K without a scene_id.
    let scene_id = pilot
        .get("scene_id")
        .and_then(Value::as_u64)
        .and_then(|id| u16::try_from(id).ok())
        .or_else(|| (state && temp == Some(4200)).then_some(PREFERRED_SCENE));
    Ok(Pilot {
        state,
        scene_id,
        temp,
    })
}

fn cloud_request(
    agent: &ureq::Agent,
    config: &CloudConfig,
    token: &str,
    url: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let authorization = format!("Bearer {token}");
    let request = match body {
        Some(body) => agent
            .post(url)
            .header("Authorization", &authorization)
            .header("User-Agent", "WiZApp/2")
            .header("WIZ-app-version", WIZ_APP_VERSION)
            .header("WiZ-mac-address", &config.mac_address)
            .send_json(body),
        None => agent
            .get(url)
            .header("Authorization", &authorization)
            .header("User-Agent", "WiZApp/2")
            .header("WIZ-app-version", WIZ_APP_VERSION)
            .call(),
    };
    let mut response = request.map_err(|error| match error {
        ureq::Error::StatusCode(code) => format!("WiZ Cloud rechazó la solicitud (HTTP {code})."),
        _ => format!("No pude completar la solicitud a WiZ Cloud: {error}"),
    })?;
    let status = response.status().as_u16();
    let value = response
        .body_mut()
        .read_json::<Value>()
        .map_err(|_| "WiZ Cloud devolvió una respuesta JSON inválida.".to_string())?;
    if !(200..300).contains(&status) {
        let code = value
            .get("error")
            .or_else(|| value.get("code"))
            .and_then(Value::as_str)
            .filter(|code| {
                matches!(
                    *code,
                    "forbidden"
                        | "unauthorized"
                        | "access_denied"
                        | "permission_denied"
                        | "not_found"
                        | "invalid_token"
                        | "token_expired"
                )
            })
            .unwrap_or("error");
        let detail = cloud_error_detail(&value, token)
            .map(|detail| format!(": {detail}"))
            .unwrap_or_default();
        return Err(format!(
            "WiZ Cloud rechazó la solicitud (HTTP {status}, {code}{detail})."
        ));
    }
    Ok(value)
}

fn cloud_error_detail(value: &Value, token: &str) -> Option<String> {
    let message = ["message", "detail", "error_description"]
        .iter()
        .find_map(|field| value.get(field).and_then(Value::as_str))?;
    let message = message.replace(token, "[redacted]");
    let detail = message
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | ':' | '.')
        })
        .take(120)
        .collect::<String>();
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    (!detail.is_empty()).then_some(detail)
}

fn cloud_command_body(action: &Action) -> Result<Value, String> {
    let mut body = json!({});
    match action {
        Action::Daylight => {
            body["state"] = json!(true);
            body["scene_id"] = json!(PREFERRED_SCENE);
        }
        Action::On => body["state"] = json!(true),
        Action::Off => body["state"] = json!(false),
        Action::Scene(scene) => {
            body["state"] = json!(true);
            body["scene_id"] = json!(scene);
        }
        Action::Temperature(temperature) => {
            body["state"] = json!(true);
            body["temperature"] = json!(temperature);
        }
        Action::Connect | Action::Refresh | Action::Exit => {
            return Err("acción no válida".to_string());
        }
    }
    Ok(body)
}

fn cloud_expected_state(action: &Action) -> Option<(bool, Option<u16>, Option<u16>)> {
    match action {
        Action::Daylight => Some((true, Some(PREFERRED_SCENE), None)),
        Action::On => Some((true, None, None)),
        Action::Off => Some((false, None, None)),
        Action::Scene(scene) => Some((true, Some(*scene), None)),
        Action::Temperature(temperature) => Some((true, None, Some(*temperature))),
        Action::Connect | Action::Refresh | Action::Exit => None,
    }
}

fn command_fields(
    action: &Action,
) -> Option<(Vec<(&'static str, Value)>, bool, Option<u16>, Option<u16>)> {
    match action {
        Action::Daylight => Some((
            vec![("state", json!(true)), ("sceneId", json!(PREFERRED_SCENE))],
            true,
            Some(PREFERRED_SCENE),
            None,
        )),
        Action::On => Some((vec![("state", json!(true))], true, None, None)),
        Action::Off => Some((vec![("state", json!(false))], false, None, None)),
        Action::Scene(scene) => Some((
            vec![("state", json!(true)), ("sceneId", json!(scene))],
            true,
            Some(*scene),
            None,
        )),
        Action::Temperature(temperature) => Some((
            vec![("state", json!(true)), ("temp", json!(temperature))],
            true,
            None,
            Some(*temperature),
        )),
        Action::Connect | Action::Refresh | Action::Exit => None,
    }
}

fn get_pilot(config: &LightConfig) -> Result<Pilot, String> {
    let result = exchange(config, "getPilot", &[])?;
    let mac = result
        .get("mac")
        .and_then(Value::as_str)
        .map(normalize_mac)
        .ok_or_else(|| "WiZ no devolvió la identidad del foco.".to_string())?;
    verify_device_mac(&config.mac, &mac)?;
    let state = result
        .get("state")
        .and_then(Value::as_bool)
        .ok_or_else(|| "WiZ no devolvió un estado válido.".to_string())?;
    let scene_id = result
        .get("sceneId")
        .and_then(Value::as_u64)
        .and_then(|id| u16::try_from(id).ok());
    let temp = result
        .get("temp")
        .and_then(Value::as_u64)
        .and_then(|temperature| u16::try_from(temperature).ok());
    Ok(Pilot {
        state,
        scene_id,
        temp,
    })
}

fn verify_device_mac(expected: &str, observed: &str) -> Result<(), String> {
    if normalize_mac(observed) == expected {
        Ok(())
    } else {
        Err("La IP configurada respondió con otro dispositivo.".to_string())
    }
}

fn exchange(config: &LightConfig, method: &str, fields: &[(&str, Value)]) -> Result<Value, String> {
    let payload = signed_request(method, fields, &config.key)?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|error| format!("No pude abrir el control local: {error}"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    socket
        .connect(SocketAddrV4::new(config.ip, DEVICE_PORT))
        .map_err(|error| format!("No pude llegar al foco: {error}"))?;
    socket
        .send(payload.as_bytes())
        .map_err(|error| format!("No pude enviar el comando: {error}"))?;
    let mut buffer = [0_u8; 4096];
    let length = socket
        .recv(&mut buffer)
        .map_err(|_| format!("El foco no respondió en {}.", config.ip))?;
    let response: Value = serde_json::from_slice(&buffer[..length])
        .map_err(|error| format!("WiZ devolvió una respuesta inválida: {error}"))?;
    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        return Err(format!("WiZ rechazó la solicitud: {error}"));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| "WiZ devolvió una respuesta incompleta.".to_string())
}

fn signed_request(method: &str, fields: &[(&str, Value)], key: &[u8]) -> Result<String, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let params = compact_params(fields, timestamp)?;
    let mut signer = HmacSha256::new_from_slice(key).map_err(|error| error.to_string())?;
    signer.update(params.as_bytes());
    let signature = STANDARD.encode(signer.finalize().into_bytes());
    Ok(format!(
        "{{\"method\":{},\"params\":{},\"hmac\":{}}}",
        serde_json::to_string(method).map_err(|error| error.to_string())?,
        params,
        serde_json::to_string(&signature).map_err(|error| error.to_string())?
    ))
}

fn compact_params(fields: &[(&str, Value)], timestamp: u64) -> Result<String, String> {
    let mut params = String::from("{");
    for (index, (name, value)) in fields.iter().enumerate() {
        if index != 0 {
            params.push(',');
        }
        params.push_str(&serde_json::to_string(name).map_err(|error| error.to_string())?);
        params.push(':');
        params.push_str(&serde_json::to_string(value).map_err(|error| error.to_string())?);
    }
    if !fields.is_empty() {
        params.push(',');
    }
    params.push_str(&format!("\"sigTs\":{timestamp}}}"));
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream, UdpSocket},
        thread,
    };

    #[test]
    fn preferred_action_turns_on_in_daylight() {
        let (fields, state, scene, temp) = command_fields(&Action::Daylight).unwrap();
        assert_eq!(fields, [("state", json!(true)), ("sceneId", json!(12))]);
        assert!(state);
        assert_eq!(scene, Some(12));
        assert_eq!(temp, None);
    }

    #[test]
    fn temperature_action_turns_on_and_verifies_the_selected_temperature() {
        let (fields, state, scene, temp) = command_fields(&Action::Temperature(4000)).unwrap();
        assert_eq!(fields, [("state", json!(true)), ("temp", json!(4000))]);
        assert!(state);
        assert_eq!(scene, None);
        assert_eq!(temp, Some(4000));
    }

    #[test]
    fn temperature_presets_stay_inside_device_range() {
        assert_eq!(
            temperature_options((2200, 6500)),
            [2200, 2700, 3500, 4000, 5000, 6500]
        );
        assert_eq!(temperature_options((3000, 4500)), [3000, 3500, 4000, 4500]);
        assert_eq!(
            parse_white_range(&json!({"white_range": [2200, 6500]})),
            Ok((2200, 6500))
        );
        assert!(parse_white_range(&json!({"white_range": [6500, 2200]})).is_err());
    }

    #[test]
    fn export_devices_are_validated_and_keep_each_light_identity() {
        let home = json!({
            "home_id": 456,
            "devices": [
                {"device_id": 123, "name": "Desk", "mac_address": "AA:BB:CC:DD:EE:01", "traits": {"is_tunable_white": true, "white_range": [2200, 6500]}},
                {"mac_address": "AA-BB-CC-DD-EE-02", "traits": {"is_tunable_white": false}}
            ]
        });
        let devices = parse_export_devices(&home).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "Desk");
        assert_eq!(devices[0].mac, "aabbccddee01");
        assert_eq!(devices[0].white_range, Some((2200, 6500)));
        assert_eq!(devices[0].device_id, Some(123));
        assert_eq!(devices[0].home_id, Some(456));
        assert_eq!(devices[1].name, "WiZ · ee02");
        assert_eq!(devices[1].white_range, None);
        assert_eq!(devices[1].device_id, None);
        assert_eq!(devices[1].home_id, Some(456));
        assert!(parse_export_devices(&json!({"devices": [{"mac_address": "invalid"}]})).is_err());
    }

    #[test]
    fn signed_parameters_preserve_wifi_protocol_order() {
        let fields = [("state", json!(true)), ("sceneId", json!(12))];
        assert_eq!(
            compact_params(&fields, 123).unwrap(),
            r#"{"state":true,"sceneId":12,"sigTs":123}"#
        );
    }

    #[test]
    fn signing_key_must_be_exactly_16_hex_bytes() {
        assert_eq!(
            parse_key("0123456789abcdef0123456789abcdef").unwrap().len(),
            16
        );
        assert!(parse_key("not-a-key").is_err());
        assert!(parse_key("0123456789abcdef0123456789abcdeg").is_err());
    }

    #[test]
    fn mac_comparison_ignores_separators_and_case() {
        assert_eq!(normalize_mac("AA:bb-cc:DD-ee-ff"), "aabbccddeeff");
    }

    #[test]
    fn refuses_to_control_a_different_device() {
        assert!(verify_device_mac("aabbccddeeff", "AA:BB:CC:DD:EE:FF").is_ok());
        assert!(verify_device_mac("aabbccddeeff", "00:11:22:33:44:55").is_err());
    }

    #[test]
    fn cloud_configuration_rejects_invalid_device_ids_and_mac_addresses() {
        let mut config = CloudConfig {
            device_id: 123,
            home_id: 456,
            mac_address: "AA:BB:CC:DD:EE:FF".to_string(),
            white_range: Some((2200, 6500)),
            oauth: None,
        };
        assert!(validate_cloud_config(&config).is_ok());
        config.device_id = 0;
        assert!(validate_cloud_config(&config).is_err());
        config.device_id = 123;
        config.home_id = 0;
        assert!(validate_cloud_config(&config).is_err());
        config.home_id = 456;
        config.mac_address = "not-a-mac".to_string();
        assert!(validate_cloud_config(&config).is_err());
        assert_eq!(parse_cloud_id(Some(&json!(123)), "device_id").unwrap(), 123);
        assert_eq!(parse_cloud_id(Some(&json!("456")), "home_id").unwrap(), 456);
        assert!(parse_cloud_id(Some(&json!("not-an-id")), "home_id").is_err());
    }

    #[test]
    fn cloud_oauth_configuration_rejects_partial_credentials() {
        assert!(parse_cloud_oauth(&json!({})).unwrap().is_none());
        assert!(
            parse_cloud_oauth(&json!({
                "google_oauth_client_id": "desktop-client-id"
            }))
            .is_err()
        );
        let config = parse_cloud_oauth(&json!({
            "google_oauth_client_id": "desktop-client-id",
            "wiz_oauth_client_id": "mobile-client-id",
            "wiz_oauth_client_secret": "mobile-client-secret"
        }))
        .unwrap()
        .unwrap();
        assert_eq!(config.google_client_id, "desktop-client-id");
        assert_eq!(config.google_client_secret, None);

        let config = parse_cloud_oauth(&json!({
            "google_oauth_client_id": "desktop-client-id",
            "google_oauth_client_secret": "desktop-client-secret",
            "wiz_oauth_client_id": "mobile-client-id",
            "wiz_oauth_client_secret": "mobile-client-secret"
        }))
        .unwrap()
        .unwrap();
        assert_eq!(
            config.google_client_secret.as_deref(),
            Some("desktop-client-secret")
        );
    }

    #[test]
    fn cloud_temperature_uses_the_mobile_api_field_and_turns_the_light_on() {
        assert_eq!(
            cloud_command_body(&Action::On).unwrap(),
            json!({"state": true})
        );
        assert_eq!(
            cloud_command_body(&Action::Off).unwrap(),
            json!({"state": false})
        );
        assert_eq!(
            cloud_command_body(&Action::Temperature(4000)).unwrap(),
            json!({"state": true, "temperature": 4000})
        );
        assert_eq!(
            cloud_expected_state(&Action::Temperature(4000)),
            Some((true, None, Some(4000)))
        );
    }

    #[test]
    fn cloud_daylight_command_authenticates_posts_and_confirms_the_state() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut state = false;
            let mut reads = 0;
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let (request, headers, body) = read_test_request(&stream);
                assert_eq!(
                    headers.get("authorization").map(String::as_str),
                    Some("Bearer mock-token")
                );
                assert_eq!(
                    headers.get("wiz-app-version").map(String::as_str),
                    Some("1.38.6")
                );
                let response = if request.starts_with("POST ") {
                    assert!(request.contains("/api/v2/light/123/pilot"));
                    assert_eq!(
                        headers.get("wiz-mac-address").map(String::as_str),
                        Some("AA:BB:CC:DD:EE:FF")
                    );
                    assert_eq!(body, json!({"state": true, "scene_id": PREFERRED_SCENE}));
                    state = true;
                    json!({"success": true})
                } else {
                    reads += 1;
                    assert!(request.contains("/api/v2/home/456"));
                    assert_eq!(body, Value::Null);
                    assert!(!headers.contains_key("wiz-mac-address"));
                    json!({"data": {"lights": [{
                        "id": 123,
                        "current_state": {
                            "status": state,
                            "scene_id": if reads < 5 { json!(0) } else { Value::Null },
                            "temperature": if reads < 5 { 6500 } else { 4200 }
                        }
                    }]}})
                };
                write_test_response(&mut stream, "200 OK", &response);
            }
        });

        let config = CloudConfig {
            device_id: 123,
            home_id: 456,
            mac_address: "AA:BB:CC:DD:EE:FF".to_string(),
            white_range: Some((2200, 6500)),
            oauth: None,
        };
        let pilot =
            perform_cloud_with_token(&config, &Action::Daylight, "mock-token", &base_url).unwrap();
        server.join().unwrap();
        assert!(pilot.state);
        assert_eq!(pilot.scene_id, Some(PREFERRED_SCENE));
        assert_eq!(pilot.temp, Some(4200));
    }

    #[test]
    fn cloud_scene_write_fails_if_scene_id_never_matches() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let (request, _, _) = read_test_request(&stream);
                let response = if request.starts_with("POST ") {
                    json!({"success": true})
                } else {
                    json!({"data": {"lights": [{
                        "id": 123,
                        "current_state": {"status": true, "temperature": 6500}
                    }]}})
                };
                write_test_response(&mut stream, "200 OK", &response);
            }
        });

        let config = CloudConfig {
            device_id: 123,
            home_id: 456,
            mac_address: "AA:BB:CC:DD:EE:FF".to_string(),
            white_range: Some((2200, 6500)),
            oauth: None,
        };
        assert!(
            perform_cloud_with_token(&config, &Action::Daylight, "mock-token", &base_url).is_err()
        );
        server.join().unwrap();
    }

    #[test]
    fn cloud_permission_errors_are_reported_without_exposing_the_token() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (request, headers, _) = read_test_request(&stream);
            assert!(request.starts_with("GET "));
            assert!(request.contains("/api/v2/home/456"));
            assert_eq!(
                headers.get("authorization").map(String::as_str),
                Some("Bearer private-test-token")
            );
            assert!(!headers.contains_key("wiz-mac-address"));
            write_test_response(
                &mut stream,
                "403 Forbidden",
                &json!({
                    "error": "forbidden",
                    "message": "blocked: Bearer private-test-token"
                }),
            );
        });

        let config = CloudConfig {
            device_id: 123,
            home_id: 456,
            mac_address: "AA:BB:CC:DD:EE:FF".to_string(),
            white_range: None,
            oauth: None,
        };
        let error = match perform_cloud_with_token(
            &config,
            &Action::Refresh,
            "private-test-token",
            &base_url,
        ) {
            Ok(_) => panic!("la respuesta 403 no debía aceptarse"),
            Err(error) => error,
        };
        server.join().unwrap();
        assert!(error.contains("HTTP 403, forbidden: blocked: Bearer redacted"));
        assert!(!error.contains("private-test-token"));
    }

    #[test]
    fn simulated_light_turns_on_in_daylight_and_confirms_state() {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, DEVICE_PORT)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server = thread::spawn(move || {
            let mut state = false;
            let mut scene_id = None;
            for _ in 0..3 {
                let mut buffer = [0_u8; 4096];
                let (length, sender) = socket.recv_from(&mut buffer).unwrap();
                let request: Value = serde_json::from_slice(&buffer[..length]).unwrap();
                let response = match request.get("method").and_then(Value::as_str).unwrap() {
                    "getPilot" => json!({"result": {
                        "mac": "AA:BB:CC:DD:EE:FF",
                        "state": state,
                        "sceneId": scene_id
                    }}),
                    "setPilot" => {
                        assert_eq!(request["params"]["state"], true);
                        assert_eq!(request["params"]["sceneId"], PREFERRED_SCENE);
                        assert!(
                            request["hmac"]
                                .as_str()
                                .is_some_and(|value| !value.is_empty())
                        );
                        state = true;
                        scene_id = Some(PREFERRED_SCENE);
                        json!({"result": {"success": true}})
                    }
                    method => panic!("método WiZ inesperado: {method}"),
                };
                socket
                    .send_to(response.to_string().as_bytes(), sender)
                    .unwrap();
            }
        });

        let config = LightConfig {
            ip: Ipv4Addr::LOCALHOST,
            mac: "aabbccddeeff".to_string(),
            key: vec![7; 16],
            white_range: None,
        };
        let pilot = perform(&config, &Action::Daylight).unwrap();
        server.join().unwrap();
        assert!(pilot.state);
        assert_eq!(pilot.scene_id, Some(PREFERRED_SCENE));
    }

    fn read_test_request(stream: &TcpStream) -> (String, HashMap<String, String>, Value) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.trim_end().split_once(':') {
                headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let body_length = headers
            .get("content-length")
            .and_then(|length| length.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; body_length];
        reader.read_exact(&mut body).unwrap();
        let body = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (request, headers, body)
    }

    fn write_test_response(stream: &mut TcpStream, status: &str, body: &Value) {
        let body = body.to_string();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }
}
