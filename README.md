# WiZ Tray

Control de focos WiZ desde la bandeja de Windows. Incluye encendido y apagado, Luz de día, escenas, temperaturas admitidas por el foco y lectura del estado. Usa el protocolo local firmado de WiZ; no depende de WiZ Cloud.

## Probar desde un clon limpio

Con Docker Desktop y Compose instalados, ejecuta en la carpeta del proyecto:

```powershell
docker compose run --build --rm build
```

El contenedor ejecuta una prueba UDP contra un foco simulado en `localhost` y compila el ejecutable Windows. Al terminar deja `dist/WiZ Tray.exe` y una configuración de ejemplo en `dist/`. La simulación valida el envío del comando y la confirmación del estado; no se conecta a un foco real.

Docker no muestra la bandeja de Windows. Ejecuta `dist/WiZ Tray.exe` en Windows para usar la interfaz.

## Conectar un foco

1. Copia `dist/wiz-tray.json` y reemplaza la IP de ejemplo por la IP actual del foco.
2. Guarda el export privado de WiZ como `dist/wiz-home.json`, junto al ejecutable. También puedes definir `WIZ_HOME_JSON` para usar otra ruta.
3. Ejecuta `dist/WiZ Tray.exe`. La aplicación lee la clave local desde el export y no la copia al ejecutable.

El export `wiz-home.json` contiene la clave privada de control del hogar. No lo subas a GitHub ni lo compartas. `wiz-home.json`, `wiz-tray.json`, `dist/`, ejecutables y accesos directos están excluidos de Git.

Se requiere Windows 10/11 de 64 bits y que el foco esté accesible en la red local. Activa la comunicación local desde la configuración de seguridad de WiZ. La app verifica la MAC del dispositivo antes de enviar comandos y consulta de nuevo el estado para confirmar cada cambio.

## Compilar directamente en Windows

Con Rust estable, `rustfmt`, el target GNU y WinLibs POSIX UCRT instalados:

```powershell
rustup target add x86_64-pc-windows-gnu
rustup component add rustfmt
.\build.ps1
```

El script ejecuta formato, pruebas y build de release. La documentación de la API cloud disponible públicamente corresponde a WiZ Pro: <https://docs.pro.wizconnected.com/>.
