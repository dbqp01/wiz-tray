//! Punto de entrada de la app de bandeja de WiZ (Tauri v2).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appstate;

use std::time::{Duration, Instant};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use wiz_tray::{state, Action};

use appstate::Shared;

/// Arranca la app: bandeja, ventana oculta y hilo de sondeo.
pub fn run() {
    tauri::Builder::default()
        .manage(appstate::shared())
        .invoke_handler(tauri::generate_handler![
            appstate::snapshot,
            appstate::perform,
            appstate::apply_scene,
            appstate::refresh,
            appstate::set_backend,
            appstate::set_show_all_scenes,
            appstate::discover,
            appstate::pick_export,
            appstate::load_export,
            appstate::save_local_setup,
            appstate::save_cloud_setup,
            appstate::save_cloud_oauth,
            appstate::clear_cloud_session_command,
            appstate::set_routine,
            appstate::start_timer,
            appstate::cancel_timer,
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            spawn_poller(app.handle().clone());
            show_window(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("WiZ Tray no pudo arrancar");
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir WiZ Tray", true, None::<&str>)?;
    let on = MenuItem::with_id(app, "on", "Encender", true, None::<&str>)?;
    let off = MenuItem::with_id(app, "off", "Apagar", true, None::<&str>)?;
    let daylight = MenuItem::with_id(app, "daylight", "Luz de día", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &separator, &on, &off, &daylight, &separator, &quit],
    )?;

    TrayIconBuilder::with_id("wiz-tray")
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray.ico"
        ))?)
        .tooltip("WiZ Tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "on" => tray_action(app, Action::On),
            "off" => tray_action(app, Action::Off),
            "daylight" => tray_action(app, Action::Daylight),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Muestra la ventana y la trae al frente.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Ejecuta una acción desde la bandeja sin bloquear la interfaz.
fn tray_action(app: &AppHandle, action: Action) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let shared = app.state::<Shared>().inner().clone();
        let (backend, connections) = {
            let guard = shared.lock().expect("estado de WiZ Tray");
            (guard.preferences.backend, guard.connections.clone())
        };
        if !connections.has(backend) {
            return;
        }
        let result =
            tauri::async_runtime::spawn_blocking(move || connections.perform(backend, &action))
                .await;
        if let Ok(Ok(pilot)) = result {
            let mut guard = shared.lock().expect("estado de WiZ Tray");
            appstate::apply_pilot(&mut guard, pilot);
        }
        appstate::publish(&app, &shared);
    });
}

/// Hilo de fondo: lee el foco, cumple el temporizador y la rutina diaria.
fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || loop {
        let shared = app.state::<Shared>().inner().clone();
        // En local el foco contesta al instante; Cloud va a la API y se consulta menos.
        let backend = shared
            .lock()
            .expect("estado de WiZ Tray")
            .preferences
            .backend;
        std::thread::sleep(match backend {
            state::Backend::Cloud => Duration::from_secs(60),
            state::Backend::Local => Duration::from_secs(4),
        });
        let (backend, connections, routine) = {
            let guard = shared.lock().expect("estado de WiZ Tray");
            (
                guard.preferences.backend,
                guard.connections.clone(),
                guard.preferences.daily_routine.clone(),
            )
        };
        if !connections.has(backend) {
            continue;
        }

        let (today, minute) = state::local_day_and_minute();
        if let Some(action) = state::routine_action_due(&routine, today, minute) {
            let _ = connections.perform(backend, &action);
            let mut guard = shared.lock().expect("estado de WiZ Tray");
            guard.preferences.daily_routine.last_run_day = Some(today);
            guard.draft.daily_routine.last_run_day = Some(today);
            let _ = state::save_preferences(&guard.preferences);
        }

        let expired = shared
            .lock()
            .expect("estado de WiZ Tray")
            .timer_due
            .is_some_and(|due| Instant::now() >= due);
        if expired {
            let _ = connections.perform(backend, &Action::Off);
            let mut guard = shared.lock().expect("estado de WiZ Tray");
            guard.timer_due = None;
            guard.status = "Apagado automático".to_string();
        }

        if let Ok(pilot) = connections.perform(backend, &Action::Refresh) {
            let mut guard = shared.lock().expect("estado de WiZ Tray");
            appstate::apply_pilot(&mut guard, pilot);
        }
        appstate::publish(&app, &shared);
    });
}

fn main() {
    run();
}

#[cfg(test)]
mod tests {
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;

    /// El JavaScript manda los argumentos en camelCase: comprueba que llegan al comando.
    #[test]
    fn los_argumentos_camel_case_llegan_al_comando() {
        let app = mock_builder()
            .manage(crate::appstate::shared())
            .invoke_handler(tauri::generate_handler![
                crate::appstate::set_show_all_scenes
            ])
            .build(tauri::test::mock_context(noop_assets()))
            .expect("app de prueba");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("ventana de prueba");

        let request = InvokeRequest {
            cmd: "set_show_all_scenes".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().expect("url"),
            body: InvokeBody::Json(serde_json::json!({ "showAll": true })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };

        let response = get_ipc_response(&webview, request).expect("respuesta del comando");
        let value = response.deserialize::<serde_json::Value>().expect("json");
        let escenas = value["scenes"].as_array().expect("lista de escenas");
        assert_eq!(escenas.len(), 28, "con showAll deben verse las 28 escenas");
    }
}
