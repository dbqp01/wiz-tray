# WiZ Tray

Control de focos WiZ desde la bandeja de Windows. Incluye encendido y apagado, Luz de día, escenas, temperaturas admitidas por el foco y lectura del estado. Permite elegir entre el protocolo local firmado y el servicio cloud que usa la app móvil.

## Probar desde un clon limpio

Con Docker Desktop y Compose instalados, ejecuta en la carpeta del proyecto:

```powershell
docker compose run --build --rm build
```

El contenedor ejecuta una prueba UDP contra un foco simulado en `localhost` y compila el ejecutable Windows. Al terminar deja `dist/WiZ Tray.exe` y una configuración de ejemplo en `dist/`. La simulación valida el envío del comando y la confirmación del estado; no se conecta a un foco real.

Docker no muestra la bandeja de Windows. Ejecuta `dist/WiZ Tray.exe` en Windows para usar la interfaz.

## Conectar un foco por red local

1. Copia `dist/wiz-tray.json` y reemplaza la IP de ejemplo por la IP actual del foco.
2. Guarda el export privado de WiZ como `dist/wiz-home.json`, junto al ejecutable. También puedes definir `WIZ_HOME_JSON` para usar otra ruta.
3. Ejecuta `dist/WiZ Tray.exe`. La aplicación lee la clave local desde el export y no la copia al ejecutable.

El export `wiz-home.json` contiene la clave privada de control del hogar. No lo subas a GitHub ni lo compartas. `wiz-home.json`, `wiz-tray.json`, `dist/`, ejecutables y accesos directos están excluidos de Git.

Se requiere Windows 10/11 de 64 bits y que el foco esté accesible en la red local. Activa la comunicación local desde la configuración de seguridad de WiZ. La app verifica la MAC del dispositivo antes de enviar comandos y consulta de nuevo el estado para confirmar cada cambio.

## Conectar por WiZ Cloud

En `wiz-tray.json`, cambia `backend` a `cloud` y completa `device_id`, `home_id`, `mac_address` y `white_range` para el foco. Esos valores son identificadores y capacidades; puedes tomarlos del export WiZ. El modo cloud no lee ni necesita la clave UDP del export.

Configura un cliente OAuth de tipo **Desktop app** en Google Cloud y agrega su ID junto con las credenciales OAuth de la app WiZ. Al elegir **Conectar WiZ Cloud con Google** en el menú, WiZ Tray abre el navegador, solicita solo `openid`, `email` y `profile`, y recibe la respuesta por un puerto local con PKCE. Elige una cuenta personal vinculada a la casa y concede el acceso que muestra Google. La app canjea el token de Google por tokens de WiZ; no requiere que pegues tokens ni contraseñas.

WiZ Tray guarda el refresh token de WiZ cifrado con DPAPI en `%LOCALAPPDATA%\WiZ Tray\cloud-token`. Queda ligado al usuario de Windows actual. La API doméstica de WiZ Cloud es privada y puede cambiar; WiZ publica una API de terceros para cuentas Pro en <https://docs.pro.wizconnected.com/>. Las pruebas automatizadas usan un servidor HTTP simulado y no validan el OAuth real ni el foco.

El `wiz-tray.json` de cloud tiene esta forma:

```json
{
  "backend": "cloud",
  "device_id": 12345678,
  "home_id": 1234567890,
  "mac_address": "AA:BB:CC:DD:EE:FF",
  "white_range": [2200, 6500],
  "google_oauth_client_id": "<client-id-de-desktop>",
  "google_oauth_client_secret": "<secreto-opcional-del-cliente-de-google>",
  "wiz_oauth_client_id": "<client-id-de-la-app-wiz>",
  "wiz_oauth_client_secret": "<client-secret-de-la-app-wiz>"
}
```

`google_oauth_client_secret` es opcional; agrégalo si Google lo exige para el cliente creado.

Si `backend` se omite, la aplicación conserva el modo `local` de versiones anteriores.

## Compilar directamente en Windows

Con Rust estable, `rustfmt`, el target GNU y WinLibs POSIX UCRT instalados:

```powershell
rustup target add x86_64-pc-windows-gnu
rustup component add rustfmt
.\build.ps1
```

El script ejecuta formato, pruebas y build de release. La documentación de la API cloud disponible públicamente corresponde a WiZ Pro: <https://docs.pro.wizconnected.com/>.
