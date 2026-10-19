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
2. Abre **WiZ Tray > Conexiones > Local** y pulsa **Buscar focos en mi red**. La app escanea la subred `/24` por UDP 38899 y muestra IP + MAC; **Usar y conectar** guarda la conexión al instante. También puedes escribir IP y MAC a mano.
3. El **token es opcional**: si importas el export `wiz-home.json`, la app firma los comandos con HMAC; si no, envía JSON sin firma. Muchos firmwares domésticos aceptan comandos sin firma (verificado en un foco real); otros más nuevos los rechazan y entonces hace falta el export.
4. Confirma que el computador y el foco estén en la misma red Wi-Fi.

El export `wiz-home.json` contiene una clave privada de control. WiZ Tray solo guarda su ruta: lee la clave localmente cuando arranca. No pegues ni subas ese archivo. Puedes guardarlo fuera de la carpeta del repositorio.

## WiZ Cloud

Cloud puede controlar la luz cuando estás fuera de casa, pero esta app no es un servicio alojado: el computador debe permanecer encendido, conectado a internet y con WiZ Tray abierta en la bandeja.

Configúralo desde **Conexiones > WiZ Cloud**:

1. Importa el export WiZ para completar hogar, foco y MAC, o escribe esos datos manualmente.
2. Introduce el Client ID de Google y las credenciales OAuth privadas de WiZ. El secreto de Google es opcional; el Client ID y el secreto privados de WiZ son necesarios y deben obtenerse por una vía autorizada. Este repositorio no los incluye.
3. Pulsa **Guardar Cloud** y después **Iniciar sesión con Google**. El navegador solicita `openid`, `email` y `profile` y vuelve a la app por `127.0.0.1`.

Google autentica la cuenta y no reemplaza las credenciales de WiZ. Un clon limpio no puede completar Cloud hasta que se proporcionen valores válidos. Las credenciales y el refresh token se protegen con Windows DPAPI y quedan ligados al perfil actual. El estado remoto se consulta cada minuto. No pegues secretos en el chat ni los subas a GitHub.

WiZ publica una API documentada para instalaciones Pro: <https://docs.pro.wizconnected.com/>. Es una integración distinta a la API doméstica usada aquí.

## Uso

- **Control**: estado confirmado, encender con Luz de día, apagar, temperatura, brillo con presets, 8 colores con selector personalizado y escenas. La vista inicial muestra seis favoritas; **Ver todas** incluye Fiesta, Halloween y otras 22 escenas. Algunos modos dependen del modelo de foco.
- **Animaciones**: el foco del panel respira, gira sus rayos y emite chispas cuando está encendido, con transición suave de color; se desactivan si Windows tiene las animaciones desactivadas.
- **Rutinas**: apaga el foco tras un intervalo o guarda una acción diaria para apagarlo o encender Luz de día a una hora local. La rutina diaria funciona mientras WiZ Tray permanezca abierta en la bandeja; Cloud necesita una sesión activa.
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
