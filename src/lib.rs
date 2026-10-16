use std::{
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

const DEVICE_PORT: u16 = 38899;
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

pub fn normalize_mac(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn perform(config: &LightConfig, action: &Action) -> Result<Pilot, String> {
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
        Action::Refresh | Action::Exit => None,
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
    use std::{net::UdpSocket, thread};

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
}
