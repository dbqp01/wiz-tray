# WiZ Tray

App ligera para controlar focos WiZ desde la bandeja de Windows: encender, apagar, elegir **Luz de día**, escenas, temperaturas y consultar el estado.

## Inicio rápido en Windows

Necesitas Windows 10/11 de 64 bits y Docker Desktop con Compose. Docker compila la app; **no la ejecuta dentro del contenedor**.

```powershell
git clone https://github.com/dbqp01/wiz-tray.git
cd wiz-tray
docker compose run --build --rm build
```

El comando deja `dist/WiZ Tray.exe` y, si aún no existe, `dist/wiz-tray.json`. La compilación también ejecuta pruebas contra un foco simulado; eso no demuestra que una bombilla real o el login cloud funcionen. Elige uno de los modos siguientes, configura el archivo JSON y después abre `dist/WiZ Tray.exe`.

## Modo local: misma red Wi-Fi

Es la ruta recomendada para empezar. No requiere Google OAuth ni WiZ Cloud.

1. En la app WiZ, activa la comunicación local en **Ajustes > Seguridad**.
2. Ve a **Integraciones > Integraciones locales > Obtener enlace** y guarda el JSON exportado como `dist/wiz-home.json`, junto al ejecutable.
3. Abre `dist/wiz-tray.json`, deja `"backend": "local"` y reemplaza `device_ip` por la IP actual de tu foco.
4. Confirma que la PC y el foco estén en la misma red local y ejecuta `dist/WiZ Tray.exe`.

La app toma la clave local y la MAC del export. `wiz-home.json` contiene una clave privada de control: no lo compartas ni lo subas a GitHub. `wiz-home.json`, `wiz-tray.json` y `dist/` están excluidos de Git.

## Modo WiZ Cloud: control remoto

La app puede enviar comandos a través del servicio cloud de WiZ, así que la PC no necesita estar en la misma red del foco. La PC sí debe estar encendida, conectada a internet y con WiZ Tray abierta.

**Antes de configurarlo, ten en cuenta:** la API doméstica cloud que usa la app es privada y no está documentada para terceros. El repositorio no incluye las credenciales OAuth de la app WiZ ni ofrece un método oficial para obtenerlas. Crear un cliente OAuth de Google por sí solo no basta. Si solo clonaste este repo y tienes una cuenta Google normal, actualmente no puedes completar el modo Cloud siguiendo únicamente estas instrucciones. WiZ sí publica documentación oficial para cuentas Pro: <https://docs.pro.wizconnected.com/>.

En esta versión la configuración se introduce editando `wiz-tray.json`; todavía no hay un formulario dentro de la app. Una pantalla de configuración simplificaría ese paso, pero no sustituye las credenciales OAuth de WiZ que faltan en el repositorio.

Si ya tienes credenciales OAuth de WiZ obtenidas por una vía autorizada, configura `dist/wiz-tray.json` así:

```json
{
  "backend": "cloud",
  "device_id": 12345678,
  "home_id": 1234567890,
  "mac_address": "AA:BB:CC:DD:EE:FF",
  "white_range": [2200, 6500],
  "google_oauth_client_id": "<client-id-de-Desktop-app>",
  "google_oauth_client_secret": "<secreto-opcional-de-Google>",
  "wiz_oauth_client_id": "<client-id-de-la-app-WiZ>",
  "wiz_oauth_client_secret": "<client-secret-de-la-app-WiZ>"
}
```

`device_id`, `home_id`, `mac_address` y `white_range` describen el foco y se pueden tomar del export WiZ. El `google_oauth_client_id` es de un cliente **Desktop app** configurado por quien ejecuta la aplicación; el secreto de Google es opcional si Google no lo exige. Los campos `wiz_oauth_*` son credenciales distintas, de WiZ. No inventes valores ni los reemplaces con los de Google.

Con la configuración completa, abre WiZ Tray y elige **Conectar WiZ Cloud con Google**. Inicia sesión con una cuenta personal vinculada a esa casa y concede los scopes `openid`, `email` y `profile`. Si la app OAuth de Google está en modo de prueba, agrega esa cuenta como usuario de prueba. El token de renovación de WiZ se guarda cifrado con Windows DPAPI en `%LOCALAPPDATA%\WiZ Tray\cloud-token` y queda ligado al perfil de Windows actual.

En modo Cloud la app no necesita la clave UDP de `wiz-home.json`, aunque puedes usar el export para consultar los identificadores del foco. Mantén `wiz-tray.json` privado: contiene configuración y secretos OAuth. No pegues contraseñas ni tokens en issues, chats o commits.

## Compilar directamente en Windows

Con Rust estable, `rustfmt`, el target GNU y WinLibs POSIX UCRT instalados:

```powershell
rustup target add x86_64-pc-windows-gnu
rustup component add rustfmt
.\build.ps1
```

El script comprueba formato, ejecuta las pruebas y compila el release para Windows. Docker Compose es la alternativa reproducible que prepara esas herramientas dentro del contenedor.

## Agentes y colaboradores

Lee [AGENTS.md](AGENTS.md) antes de modificar el código. Contiene el flujo recomendado de instalación y las limitaciones conocidas para no confundir una prueba simulada con el control de una bombilla real.

