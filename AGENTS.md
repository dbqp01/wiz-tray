# Instrucciones para agentes de código

Lee primero `README.md` y `git status --short`. Conserva los cambios existentes del usuario. Este proyecto es una app de bandeja de Windows escrita en Rust 2024; `Cargo.toml` y `Cargo.lock` son la fuente de dependencias.

## Si ayudas a alguien a instalarla

1. Confirma que la persona usa Windows 10/11 de 64 bits. Docker sirve para compilar, no para mostrar ni ejecutar la bandeja.
2. Explica que `docker compose run --build --rm build` produce `dist/WiZ Tray.exe` y prueba contra un foco simulado. No presentes esa prueba como validación de una bombilla real.
3. Pregunta qué necesita: control local en la misma red o control remoto por WiZ Cloud.
4. Para el modo local, guía la exportación local desde la app WiZ, la copia a `dist/wiz-home.json` y la configuración de `device_ip`. No pidas que pegue o suba el export: contiene una clave privada.
5. Para Cloud, lee el aviso de `README.md` antes de dar pasos. El flujo doméstico usa endpoints privados; el repo no incluye credenciales OAuth de la app WiZ ni documenta una fuente oficial para conseguirlas. El cliente OAuth de Google no reemplaza esas credenciales. Si faltan, explica el bloqueo con claridad; no inventes valores ni prometas que un clon limpio funciona en Cloud.
6. Nunca pidas contraseñas, códigos OAuth, access tokens o refresh tokens en el chat. Indica que el login debe abrirse en el navegador del usuario y que la configuración sensible se guarda solo localmente.
7. Si la persona quiere la API oficialmente documentada, remite a la documentación de WiZ Pro enlazada en el README y explica que es una ruta distinta.

## Interfaz de configuración

- Prioriza un formulario sencillo en la app para configurar el foco y OAuth, en vez de pedir a cada usuario que edite JSON a mano.
- Valida los campos antes de guardar y deja los valores sensibles en almacenamiento local protegido por Windows (DPAPI o Credential Manager); no los escribas en logs ni los muestres después de guardarlos.
- La interfaz no resuelve la falta de credenciales OAuth de la app WiZ: nunca las inventes ni las presentes como provistas por Google.

## Cambios y verificación

- Antes de editar, revisa `Cargo.toml`, `Cargo.lock`, `build.ps1`, `compose.yaml` y los llamadores afectados. Mantén el cambio pequeño y no reviertas ediciones previas.
- `src/windows_app.rs` carga `wiz-tray.json` desde la carpeta del ejecutable y construye la bandeja. `src/lib.rs` implementa validación y comandos locales/cloud. `src/cloud_auth.rs` maneja Google OAuth, el intercambio WiZ y el token protegido con DPAPI.
- Mantén fuera de Git `wiz-tray.json`, `wiz-home.json`, tokens, credenciales OAuth, claves y ejecutables. Revisa `.gitignore` antes de preparar un commit; nunca incluyas valores de secretos en mensajes, logs o pruebas.
- El flujo reproducible de Docker es `docker compose run --build --rm build`: comprueba el código con pruebas simuladas y compila el `.exe`. En Windows, `build.ps1` comprueba formato, ejecuta pruebas y compila el release. Distingue siempre estas comprobaciones de una prueba real con WiZ Cloud o una bombilla física.
- Al reportar el resultado, indica qué se probó realmente y qué depende de la red, credenciales OAuth o hardware del usuario.

