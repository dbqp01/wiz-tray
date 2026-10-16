use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::PathBuf,
    ptr::{null, null_mut},
    thread,
    time::{Duration, Instant},
};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, CRYPT_INTEGER_BLOB,
        CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

use crate::{CloudConfig, CloudOAuthConfig};

const WIZ_TOKEN_URL: &str = "https://api.wiz.world/oauth/v2/token";
const GOOGLE_SCOPE: &str = "openid email profile";
const WIZ_SCOPE: &str = "wiz_android_v2";
const WIZ_GOOGLE_GRANT: &str = "urn:google_v2";
const TOKEN_PREFIX: &[u8] = b"WiZ Tray token v1\n";
const TOKEN_FILE: &str = "cloud-token";
const TOKEN_LIMIT: usize = 16 * 1024;
const CALLBACK_WAIT: Duration = Duration::from_secs(300);

pub(super) fn connect(config: &CloudConfig) -> Result<String, String> {
    let oauth = oauth_config(config)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "No pude abrir el retorno local para Google OAuth.".to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "No pude preparar el retorno local para Google OAuth.".to_string())?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}",
        listener
            .local_addr()
            .map_err(|_| "No pude preparar el retorno local para Google OAuth.".to_string())?
            .port()
    );
    let state = random_urlsafe(32)?;
    let verifier = random_urlsafe(32)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        url_encode(&oauth.google_client_id),
        url_encode(&redirect_uri),
        url_encode(GOOGLE_SCOPE),
        state,
        challenge,
    );
    open_browser(&auth_url)?;

    let code = wait_for_code(&listener, &state)?;
    let google_token = exchange_google_code(oauth, &code, &verifier, &redirect_uri)?;
    let token = exchange_wiz_token(oauth, &google_token_fields(&google_token))?;
    let access_token = required_string(&token, "access_token", "WiZ")?;
    let refresh_token = required_string(&token, "refresh_token", "WiZ")?;
    save_refresh_token(&refresh_token)?;
    Ok(access_token.to_owned())
}

pub(super) fn access_token(config: &CloudConfig) -> Result<String, String> {
    let oauth = oauth_config(config)?;
    let refresh_token = load_refresh_token()?;
    let token = exchange_wiz_token(oauth, &refresh_token_fields(oauth, &refresh_token))?;
    let access_token = required_string(&token, "access_token", "WiZ")?;
    if let Some(next_refresh_token) = token.get("refresh_token").and_then(Value::as_str) {
        if next_refresh_token != refresh_token {
            save_refresh_token(next_refresh_token)?;
        }
    }
    Ok(access_token.to_owned())
}

fn oauth_config(config: &CloudConfig) -> Result<&CloudOAuthConfig, String> {
    let oauth = config.oauth.as_ref().ok_or_else(|| {
        "Falta la configuración OAuth en wiz-tray.json; agrega los tres campos google_oauth_client_id, wiz_oauth_client_id y wiz_oauth_client_secret.".to_string()
    })?;
    if oauth.google_client_id.trim().is_empty()
        || oauth.wiz_client_id.trim().is_empty()
        || oauth.wiz_client_secret.trim().is_empty()
    {
        return Err("La configuración OAuth de wiz-tray.json está incompleta.".to_string());
    }
    Ok(oauth)
}

fn exchange_google_code(
    oauth: &CloudOAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    let agent = auth_agent();
    let mut fields = vec![
        ("client_id", oauth.google_client_id.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(client_secret) = oauth.google_client_secret.as_deref() {
        fields.push(("client_secret", client_secret));
    }
    let response = agent
        .post("https://oauth2.googleapis.com/token")
        .send_form(fields)
        .map_err(|error| auth_error("Google", error))?;
    let status = response.status().as_u16();
    let token = response
        .into_body()
        .read_json::<Value>()
        .map_err(|_| "Google devolvió una respuesta de autenticación inválida.".to_string())?;
    if !(200..300).contains(&status) {
        return Err(auth_status_error("Google", status, &token));
    }
    required_string(&token, "access_token", "Google").map(str::to_owned)
}

fn exchange_wiz_token(oauth: &CloudOAuthConfig, fields: &[(&str, &str)]) -> Result<Value, String> {
    let credentials = STANDARD.encode(format!(
        "{}:{}",
        oauth.wiz_client_id, oauth.wiz_client_secret
    ));
    let response = auth_agent()
        .post(WIZ_TOKEN_URL)
        .header("Authorization", &format!("Basic {credentials}"))
        .send_form(fields.iter().copied())
        .map_err(|error| auth_error("WiZ Cloud", error))?;
    let status = response.status().as_u16();
    let value = response
        .into_body()
        .read_json::<Value>()
        .map_err(|_| "WiZ Cloud devolvió una respuesta de autenticación inválida.".to_string())?;
    if !(200..300).contains(&status) {
        return Err(auth_status_error("WiZ Cloud", status, &value));
    }
    Ok(value)
}

fn google_token_fields(token: &str) -> [(&'static str, &str); 3] {
    [
        ("grant_type", WIZ_GOOGLE_GRANT),
        ("scope", WIZ_SCOPE),
        ("token", token),
    ]
}

fn refresh_token_fields<'a>(
    oauth: &'a CloudOAuthConfig,
    refresh_token: &'a str,
) -> [(&'static str, &'a str); 4] {
    [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &oauth.wiz_client_id),
        ("client_secret", &oauth.wiz_client_secret),
    ]
}

fn auth_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

fn auth_status_error(service: &str, status: u16, body: &Value) -> String {
    let code = body
        .get("error")
        .and_then(Value::as_str)
        .filter(|code| {
            matches!(
                *code,
                "invalid_grant"
                    | "invalid_client"
                    | "invalid_request"
                    | "unauthorized_client"
                    | "unsupported_grant_type"
            )
        })
        .unwrap_or("error");
    let detail = auth_status_detail(body)
        .map(|detail| format!(": {detail}"))
        .unwrap_or_default();
    format!("{service} rechazó la autenticación (HTTP {status}, {code}{detail}).")
}

fn auth_status_detail(body: &Value) -> Option<String> {
    let description = body.get("error_description")?.as_str()?;
    let detail = description
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | ':' | '.')
        })
        .take(120)
        .collect::<String>();
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    (!detail.trim().is_empty()).then_some(detail)
}

fn auth_error(service: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => {
            format!("{service} rechazó la autenticación (HTTP {code}).")
        }
        _ => format!("No pude completar la autenticación con {service}."),
    }
}

fn required_string<'a>(value: &'a Value, field: &str, service: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= TOKEN_LIMIT)
        .ok_or_else(|| format!("{service} no devolvió un {field} válido."))
}

fn random_urlsafe(size: usize) -> Result<String, String> {
    let mut bytes = vec![0; size];
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err("Windows no pudo generar valores seguros para Google OAuth.".to_string());
    }
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn url_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn open_browser(url: &str) -> Result<(), String> {
    let operation: Vec<u16> = "open\0".encode_utf16().collect();
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err("No pude abrir el navegador para iniciar sesión con Google.".to_string());
    }
    Ok(())
}

fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    let deadline = Instant::now() + CALLBACK_WAIT;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let request = read_callback_request(&mut stream);
                match request.and_then(|request| callback_code(&request, expected_state)) {
                    Ok(code) => {
                        let _ = write_callback_response(
                            &mut stream,
                            "200 OK",
                            "WiZ Tray ya puede cerrarse. Regresa a la aplicación.",
                        );
                        return Ok(code);
                    }
                    Err(error) => {
                        let _ = write_callback_response(
                            &mut stream,
                            "400 Bad Request",
                            "No se pudo completar el inicio de sesión. Regresa a WiZ Tray e inténtalo de nuevo.",
                        );
                        if error == "Google canceló el inicio de sesión." {
                            return Err(error);
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return Err("Falló el retorno local de Google OAuth.".to_string()),
        }
    }
    Err("El inicio de sesión con Google expiró; vuelve a intentarlo desde WiZ Tray.".to_string())
}

fn read_callback_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0; 1024];
    while request.len() < 8192 {
        let count = stream
            .read(&mut buffer)
            .map_err(|_| "El retorno local de Google OAuth no se pudo leer.".to_string())?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request)
        .map_err(|_| "El retorno local de Google OAuth no era texto válido.".to_string())
}

fn callback_code(request: &str, expected_state: &str) -> Result<String, String> {
    let mut request_line = request
        .lines()
        .next()
        .ok_or_else(|| "El retorno de Google OAuth no era válido.".to_string())?
        .split_whitespace();
    let method = request_line.next();
    let target = request_line
        .next()
        .filter(|target| target.starts_with("/?"))
        .ok_or_else(|| "El retorno de Google OAuth no era válido.".to_string())?;
    if method != Some("GET") {
        return Err("El retorno de Google OAuth no era válido.".to_string());
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in target[2..]
        .split('&')
        .filter_map(|part| part.split_once('='))
    {
        let key = url_decode(key)?;
        let value = url_decode(value)?;
        match key.as_str() {
            "code" if code.is_none() => code = Some(value),
            "state" if state.is_none() => state = Some(value),
            "error" if error.is_none() => error = Some(value),
            "code" | "state" | "error" => {
                return Err("El retorno de Google OAuth tenía parámetros repetidos.".to_string());
            }
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err("El retorno de Google OAuth no superó la verificación.".to_string());
    }
    if error.is_some() {
        return Err("Google canceló el inicio de sesión.".to_string());
    }
    code.filter(|code| {
        !code.is_empty() && code.len() <= TOKEN_LIMIT && !code.chars().any(char::is_control)
    })
    .ok_or_else(|| "Google no devolvió un código de autorización válido.".to_string())
}

fn url_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1])?;
                let low = hex_value(bytes[index + 2])?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            b'%' => return Err("El retorno de Google OAuth no era válido.".to_string()),
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded)
        .map_err(|_| "El retorno de Google OAuth no era texto válido.".to_string())
}

fn hex_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("El retorno de Google OAuth no era válido.".to_string()),
    }
}

fn write_callback_response(
    stream: &mut TcpStream,
    status: &str,
    message: &str,
) -> std::io::Result<()> {
    let body =
        format!("<!doctype html><meta charset=\"utf-8\"><title>WiZ Tray</title><p>{message}</p>");
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn token_path() -> Result<PathBuf, String> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("WiZ Tray").join(TOKEN_FILE))
        .ok_or_else(|| "Windows no expuso la carpeta local para guardar la sesión WiZ.".to_string())
}

fn save_refresh_token(token: &str) -> Result<(), String> {
    if token.is_empty() || token.len() > TOKEN_LIMIT || token.chars().any(char::is_control) {
        return Err("WiZ devolvió un refresh token inválido.".to_string());
    }
    let mut plaintext = TOKEN_PREFIX.to_vec();
    plaintext.extend_from_slice(token.as_bytes());
    let encrypted = dpapi_protect(&plaintext)?;
    let path = token_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| "No pude preparar el almacenamiento local de WiZ.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "No pude preparar el almacenamiento local de WiZ.".to_string())?;
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp)
        .map_err(|_| "No pude guardar de forma segura la sesión WiZ.".to_string())?;
    file.write_all(&encrypted)
        .and_then(|()| file.sync_all())
        .map_err(|_| "No pude guardar de forma segura la sesión WiZ.".to_string())?;
    drop(file);
    fs::rename(&temp, &path)
        .map_err(|_| "No pude actualizar de forma segura la sesión WiZ.".to_string())
}

fn load_refresh_token() -> Result<String, String> {
    let path = token_path()?;
    let encrypted = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "WiZ Cloud aún no está conectado. Elige «Conectar WiZ Cloud con Google» en el menú."
                .to_string()
        } else {
            "No pude leer la sesión WiZ guardada; vuelve a conectar la cuenta.".to_string()
        }
    })?;
    if encrypted.is_empty() || encrypted.len() > TOKEN_LIMIT {
        return Err(
            "La sesión WiZ guardada no es válida; vuelve a conectar la cuenta.".to_string(),
        );
    }
    let plaintext = dpapi_unprotect(&encrypted)?;
    let token = plaintext
        .strip_prefix(TOKEN_PREFIX)
        .and_then(|token| std::str::from_utf8(token).ok())
        .filter(|token| !token.is_empty() && token.len() <= TOKEN_LIMIT)
        .ok_or_else(|| {
            "La sesión WiZ guardada no es válida; vuelve a conectar la cuenta.".to_string()
        })?;
    Ok(token.to_string())
}

fn dpapi_protect(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err("Windows no pudo cifrar la sesión WiZ.".to_string());
    }
    if output.pbData.is_null() || output.cbData == 0 {
        unsafe { LocalFree(output.pbData.cast()) };
        return Err("Windows no pudo cifrar la sesión WiZ.".to_string());
    }
    let encrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(encrypted)
}

fn dpapi_unprotect(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err(
            "Windows no pudo descifrar la sesión WiZ; vuelve a conectar la cuenta.".to_string(),
        );
    }
    if output.pbData.is_null() || output.cbData == 0 {
        unsafe { LocalFree(output.pbData.cast()) };
        return Err(
            "Windows no pudo descifrar la sesión WiZ; vuelve a conectar la cuenta.".to_string(),
        );
    }
    let plaintext =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_rejects_wrong_state_and_accepts_only_a_valid_code() {
        let request = "GET /?code=auth%2Fcode&state=expected HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(callback_code(request, "expected").unwrap(), "auth/code");
        assert!(callback_code(request, "different").is_err());
        assert!(callback_code("GET /?code=auth HTTP/1.1\r\n\r\n", "expected").is_err());
    }

    #[test]
    fn wiz_oauth_forms_match_the_mobile_grants() {
        let oauth = CloudOAuthConfig {
            google_client_id: "google-client".into(),
            google_client_secret: Some("google-secret".into()),
            wiz_client_id: "wiz-client".into(),
            wiz_client_secret: "wiz-secret".into(),
        };
        assert_eq!(
            google_token_fields("google-token"),
            [
                ("grant_type", "urn:google_v2"),
                ("scope", "wiz_android_v2"),
                ("token", "google-token"),
            ]
        );
        assert_eq!(
            refresh_token_fields(&oauth, "refresh-token"),
            [
                ("grant_type", "refresh_token"),
                ("refresh_token", "refresh-token"),
                ("client_id", "wiz-client"),
                ("client_secret", "wiz-secret"),
            ]
        );
    }

    #[test]
    fn oauth_errors_report_safe_provider_details() {
        let error = auth_status_error(
            "Google",
            400,
            &serde_json::json!({
                "error": "invalid_request",
                "error_description": "Missing required parameter: code_verifier; ignore \u{0001} this value",
            }),
        );
        assert_eq!(
            error,
            "Google rechazó la autenticación (HTTP 400, invalid_request: Missing required parameter: code_verifier ignore this value)."
        );

        let error = auth_status_error(
            "Google",
            400,
            &serde_json::json!({
                "error": "unknown",
            }),
        );
        assert_eq!(error, "Google rechazó la autenticación (HTTP 400, error).");
    }

    #[test]
    fn dpapi_encrypts_and_decrypts_token_data() {
        let protected = dpapi_protect(b"test token").unwrap();
        assert_ne!(protected, b"test token");
        assert_eq!(dpapi_unprotect(&protected).unwrap(), b"test token");
    }
}
