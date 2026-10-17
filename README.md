# WiZ Tray

Aplicación de escritorio para controlar focos WiZ desde Windows. Tiene panel visual, acceso desde la bandeja, encendido y apagado, Luz de día, escenas, temperatura de blanco y apagado automático.

## Descargar y ejecutar

Necesitas Windows 10/11 de 64 bits. La compilación reproducible usa Docker Desktop con Compose:

```powershell
git clone https://github.com/dbqp01/wiz-tray.git
cd wiz-tray
docker compose run --build --rm build
& '.\dist\WiZ Tray.exe'
```

El ejecutable aparece en `dist/WiZ Tray.exe`. La app no se ejecuta dentro del contenedor. Al cerrar la ventana queda en la bandeja; usa **Salir de WiZ Tray** en el menú del icono para cerrarla por completo.

En el primer inicio, abre **Conexiones** y configura uno o ambos modos. Los datos de configuración se guardan en `%LOCALAPPDATA%\WiZ Tray\settings.json`. Las credenciales OAuth se cifran con Windows DPAPI en el mismo perfil; no se guardan en el repositorio ni se incluyen en logs.

## Control local

Control local es la opción recomendada si usas la app en casa.

1. En la app WiZ, activa **Ajustes > Seguridad > comunicación local**.
2. Abre **WiZ Tray > Conexiones > Local** y elige **Integraciones > Integraciones locales > Obtener enlace** exportado por la app WiZ.
3. Selecciona el foco del export, escribe su IP local y un nombre, y pulsa **Guardar y probar**.
4. Confirma que el computador y el foco estén en la misma red Wi-Fi.

El export `wiz-home.json` contiene una clave privada de control. WiZ Tray solo guarda su ruta: lee la clave localmente cuando arranca. No pegues ni subas ese archivo. Puedes guardarlo fuera de la carpeta del repositorio.

## WiZ Cloud

Cloud puede controlar la luz cuando estás fuera de casa, pero esta app no es un servicio alojado: el computador debe permanecer encendido, conectado a internet y con WiZ Tray abierta en la bandeja.

La API doméstica de WiZ que usa el flujo móvil no está documentada para terceros y WiZ Tray no contiene las credenciales OAuth privadas de WiZ. En **Conexiones > WiZ Cloud** puedes importar el mismo export para rellenar automáticamente el foco y el hogar, o introducir esos IDs a mano. Las credenciales OAuth de Google y WiZ se guardan allí tras obtenerlas por una vía autorizada. Google inicia sesión en el navegador y vuelve a la app por `127.0.0.1`; sus credenciales no reemplazan las de WiZ. Un clon limpio no puede completar Cloud hasta que se proporcionen esos valores. No inventes credenciales ni publiques secretos.

Una vez configurado, el inicio de sesión solicita `openid`, `email` y `profile`. El refresh token y las credenciales OAuth se protegen con DPAPI y quedan ligados al perfil de Windows actual. El estado remoto se consulta cada minuto para evitar llamadas innecesarias.

WiZ publica una API documentada para instalaciones Pro: <https://docs.pro.wizconnected.com/>. Es una integración distinta a la API doméstica usada aquí.

## Uso

- **Control**: estado confirmado, encender con Luz de día, apagar, escenas y temperatura.
- **Rutinas**: programa un apagado único; funciona mientras la app esté en la bandeja.
- **Bandeja**: abre el panel o cambia rápidamente encendido, escenas y temperaturas.
- **Local y Cloud**: guarda ambos modos y cambia entre ellos desde el encabezado del panel.

## Desarrollo y verificación

Con Docker:

```powershell
docker compose run --build --rm build
```

En Windows, con Rust estable, rustfmt y WinLibs POSIX UCRT:

```powershell
rustup target add x86_64-pc-windows-gnu
rustup component add rustfmt
.\build.ps1
```

Ambos flujos ejecutan pruebas de protocolo contra un foco simulado y compilan el ejecutable. No verifican una bombilla física, el inicio de sesión real de WiZ Cloud ni una cuenta de Google.

Lee [AGENTS.md](AGENTS.md) antes de modificar el código. El archivo resume setup, secretos y límites de verificación.
