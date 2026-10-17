# Instrucciones para agentes de código

Lee primero `README.md` y `git status --short`. Conserva los cambios existentes del usuario. Este proyecto es una app de bandeja de Windows escrita en Rust 2024; `Cargo.toml` y `Cargo.lock` son la fuente de dependencias.

## Si ayudas a alguien a instalarla

1. Confirma que la persona usa Windows 10/11 de 64 bits. Docker compila la app, pero no ejecuta ni muestra la bandeja.
2. `docker compose run --build --rm build` produce `dist/WiZ Tray.exe` y ejecuta pruebas contra un foco simulado. No lo presentes como validación de una bombilla real.
3. La app ofrece configuración guiada en **Conexiones** para Local y WiZ Cloud. Puede guardar ambos modos.
4. Para Local, guía la exportación desde la app WiZ y la selección de `wiz-home.json` en la app. No pidas que pegue o suba el export: contiene una clave privada.
5. Cloud usa endpoints domésticos privados. El repo no incluye credenciales OAuth privadas de la app WiZ ni documenta una fuente oficial para obtenerlas. Google OAuth no reemplaza esas credenciales. Un clon limpio no completa Cloud hasta que la persona proporcione credenciales obtenidas por una vía autorizada.
6. Nunca pidas contraseñas, códigos OAuth, access tokens, refresh tokens ni exports privados por chat. El login se abre en el navegador y credenciales OAuth y tokens se cifran localmente con Windows DPAPI.
7. Para una API documentada, remite a la documentación de WiZ Pro del README y aclara que es una ruta distinta.

## Interfaz de configuración

- Prioriza un formulario sencillo en la app para configurar el foco y OAuth, en vez de pedir a cada usuario que edite JSON a mano.
- Valida los campos antes de guardar y deja los valores sensibles en almacenamiento local protegido por Windows (DPAPI o Credential Manager); no los escribas en logs ni los muestres después de guardarlos.
- La interfaz no resuelve la falta de credenciales OAuth de la app WiZ: nunca las inventes ni las presentes como provistas por Google.

## Cambios y verificación

- Antes de editar, revisa `Cargo.toml`, `Cargo.lock`, `build.ps1`, `compose.yaml` y los llamadores afectados. Mantén el cambio pequeño y no reviertas ediciones previas.
- `src/windows_app.rs` implementa el dashboard, los formularios de configuración y la bandeja. Guarda preferencias sin secretos en `%LOCALAPPDATA%\WiZ Tray\settings.json`. `src/lib.rs` implementa validación y comandos locales/cloud. `src/cloud_auth.rs` maneja Google OAuth, el intercambio WiZ y el almacenamiento DPAPI.
- Mantén fuera de Git `wiz-tray.json`, `wiz-home.json`, tokens, credenciales OAuth, claves y ejecutables. Revisa `.gitignore` antes de preparar un commit; nunca incluyas valores de secretos en mensajes, logs o pruebas.
- El flujo reproducible de Docker es `docker compose run --build --rm build`: comprueba el código con pruebas simuladas y compila el `.exe`. En Windows, `build.ps1` comprueba formato, ejecuta pruebas y compila el release. Distingue siempre estas comprobaciones de una prueba real con WiZ Cloud o una bombilla física.
- Al reportar el resultado, indica qué se probó realmente y qué depende de la red, credenciales OAuth o hardware del usuario.

