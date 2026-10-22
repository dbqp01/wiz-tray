# WiZ Tray — tres direcciones para elegir

Desktop Read: Windows-first; utilidad de bandeja para tareas breves; densidad estándar; ratón y teclado. Evidencia: interfaz egui existente y revisión del ejecutable en esta sesión. Ventana objetivo: control de un foco encendido con brillo 65 %, blanco cálido 2700 K y escena Lectura. Los valores y el nombre del foco son ejemplos, no una lectura de hardware.

Direcciones:
- A / Fluent: ventana principal 1000 × 700, navegación lateral estrecha, controles en filas y favoritos en un panel secundario. A menos de 720 px la navegación pasa arriba y favoritos debajo. Recomendada para equilibrar aspecto Windows y configuración completa.
- B / Compacto: utilidad centrada de 480 × 660, navegación superior, controles esenciales y escenas en una sola columna. En una ventana grande conserva un ancho de lectura reducido. Es la dirección adecuada si se usa principalmente desde la bandeja.
- C / Estudio: ventana 1100 × 720, selector de escenas a la izquierda y ajustes precisos a la derecha. A menos de 720 px el selector se vuelve una tira horizontal. Prima explorar colores y ambientes.

Todos muestran el mismo flujo, con encendido, brillo, temperatura, color y escenas. Rutinas y conexiones tienen superficies de demostración accesibles. Los controles solo cambian el prototipo; no hay conexión con focos, archivos privados o servicios. La barra de título dibujada representa la estructura; la implementación mantendrá los controles reales de Windows. Las superficies son opacas: no se simula Mica ni Acrylic.

Se usan prototipos HTML renderizados en lugar de ImageGen, que no está disponible en esta sesión. Esto permite comprobar adaptación y controles. No implica migrar la app a tecnologías web.

## Skills y procedencia

Instaladas en C:/Users/akimi/.claude/skills: desktop-taste, desktop-design-read, desktop-art-direction, desktop-native-feel, desktop-layout-composition y desktop-visual-draft. Fuente comunitaria MIT: https://github.com/MC-Archived/Desktop-Taste-Skills, revisión 0fba492397629c79909380255199d3972c7ff5a0. Cada copia conserva LICENSE y SOURCE.json. Se leyeron sus guías para esta exploración; una sesión nueva puede ser necesaria para que aparezcan como comandos.

La guía oficial winui-design de Microsoft se revisó y no se instaló: su ámbito explícito es WinUI 3, mientras esta app usa egui. Referencia: https://github.com/microsoft/winappCli/tree/main/plugins/winui. Diseño Windows: https://learn.microsoft.com/en-us/windows/apps/design/.

## Segunda exploración tras feedback

El usuario rechazó la primera tanda. Las propuestas se revisaron: A usa superficies cálidas y navegación estrecha; B combina un panel oscuro compacto de 540 px con escenas laterales; C usa un espacio de estudio oscuro con escenas a la izquierda y controles a la derecha. Se redujo el espacio y la altura respecto de las primeras versiones. Los tamaños indicados en la cabecera son objetivos conceptuales, no medidas exactas del HTML.

Se capturaron los tres enfoques a 1280 y 640 px de viewport sin desbordamiento horizontal del documento. A 640 px las alturas de ventana son aproximadamente A 927, B 628 y C 799 px: A todavía exige desplazamiento vertical y no es el enfoque recomendado para ventanas bajas. Se comprobó actualización de brillo, encendido, selección de escenas y navegación de demostración mediante eventos DOM; no equivale a una prueba de teclado o hardware. Se inspeccionaron las capturas. Recomendación revisada: B para el uso desde bandeja, por contener los ajustes y escenas en una ventana menor.

Selección pendiente del usuario. La implementación de la app queda fuera de esta exploración hasta elegir un enfoque.
