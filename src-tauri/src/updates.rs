//! Actualizaciones de WiZ Tray desde GitHub Releases, verificadas con la firma de la app.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Lo que la tarjeta de Ajustes necesita para decir si hay una versión nueva.
#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub available: Option<String>,
    pub notes: Option<String>,
}

/// Progreso de la descarga, en bytes; `total` falta si el servidor no lo indica.
#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Pregunta a GitHub si hay una versión más nueva que la instalada.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let update = app
        .updater()
        .map_err(|error| format!("No pude preparar el actualizador: {error}"))?
        .check()
        .await
        .map_err(|error| format!("No pude buscar actualizaciones: {error}"))?;
    Ok(UpdateInfo {
        current: current_version(&app),
        available: update.as_ref().map(|update| update.version.clone()),
        notes: update.and_then(|update| update.body),
    })
}

/// Descarga la versión nueva, comprueba su firma y abre el instalador.
/// En Windows el instalador cierra la app, la reemplaza y vuelve a abrirla.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .updater()
        .map_err(|error| format!("No pude preparar el actualizador: {error}"))?
        .check()
        .await
        .map_err(|error| format!("No pude buscar actualizaciones: {error}"))?
        .ok_or_else(|| "Ya tienes la última versión.".to_string())?;
    let mut done = 0_u64;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = app.emit("update-progress", Progress { done, total });
            },
            || {},
        )
        .await
        .map_err(|error| format!("No pude instalar la actualización: {error}"))?;
    app.restart()
}
