/* Interfaz de WiZ Tray: la ventana toma el color y el brillo reales del foco. */

const bridge = window.__TAURI__
  ? {
      invoke: (command, args) => window.__TAURI__.core.invoke(command, args),
      listen: (event, handler) =>
        window.__TAURI__.event.listen(event, (payload) => handler(payload.payload)),
    }
  : null;

const $ = (id) => document.getElementById(id);

/* Windows 11 (platformVersion >= 13) pinta Mica detrás de la ventana transparente;
   en Windows 10 se mantiene el fondo sólido para que no se vea el escritorio. */
navigator.userAgentData
  ?.getHighEntropyValues(["platformVersion"])
  .then(({ platformVersion }) => {
    if (bridge && parseInt(platformVersion, 10) >= 13) document.documentElement.classList.add("mica");
  })
  .catch(() => {});

const AMBER = [245, 181, 68];

let state = null;
let timerSeconds = null;
let view = null;

/* ---------- color ---------- */

function kelvinToRgb(kelvin) {
  const k = Math.max(1000, Math.min(12000, kelvin || 4000)) / 100;
  let r;
  let g;
  let b;
  if (k <= 66) {
    r = 255;
    g = 99.4708025861 * Math.log(k) - 161.1195681661;
  } else {
    r = 329.698727446 * Math.pow(k - 60, -0.1332047592);
    g = 288.1221695283 * Math.pow(k - 60, -0.0755148492);
  }
  if (k >= 66) b = 255;
  else if (k <= 19) b = 0;
  else b = 138.5177312231 * Math.log(k - 10) - 305.0447927307;
  return [r, g, b].map((value) => Math.max(0, Math.min(255, Math.round(value))));
}

function sceneById(sceneId) {
  // En modo navegador las escenas no favoritas solo existen en el catálogo de prueba.
  return [...(state?.scenes ?? []), ...(state?.favorites ?? []), ...(bridge ? [] : CATALOG)].find(
    (candidate) => candidate.id === sceneId,
  );
}

function liveColor() {
  const pilot = state?.pilot;
  if (!pilot) return AMBER;
  if (pilot.color) return pilot.color;
  const scene = sceneById(pilot.scene_id)?.color;
  if (scene) return scene;
  return kelvinToRgb(pilot.temp ?? state?.temperature ?? 4000);
}

function luminance([r, g, b]) {
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

function toHex([r, g, b]) {
  return `#${[r, g, b].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function hexToRgb(hex) {
  const value = hex.replace("#", "");
  return [0, 2, 4].map((offset) => parseInt(value.slice(offset, offset + 2), 16));
}

/* Escena que pinta la luz ahora: solo cuando el foco está en una escena (no en color ni en blanco). */
function liveScene() {
  const pilot = state?.pilot;
  if (!pilot?.state || pilot.color || !pilot.scene_id) return null;
  return sceneById(pilot.scene_id) ?? { id: pilot.scene_id };
}

/* Pinta la luz: color real y potencia (0 apagado, 0.15–1 según el brillo).
   `scene` elige la paleta y el bucle de la escena; null pinta solo ese color. */
function paint(rgb, power, scene = liveScene()) {
  const root = document.documentElement.style;
  root.setProperty("--bulb", `rgb(${rgb.join(" ")})`);
  root.setProperty("--power", power.toFixed(2));
  const target = scene ? { id: scene.id, rgb: scene.color ?? rgb, animated: scene.animated } : { rgb };
  // El texto se elige sobre el cuerpo de la paleta a la potencia que se ve.
  const shown = window.Light.set(target, power);
  root.setProperty("--on-light", shown > 0.42 ? "#141210" : "#ffffff");
  root.setProperty("--on-light-2", shown > 0.42 ? "rgba(20, 18, 16, 0.66)" : "rgba(255, 255, 255, 0.7)");
  const accent = power ? rgb.map((value) => Math.round(value + (255 - value) * 0.28)) : AMBER;
  root.setProperty("--accent", `rgb(${accent.join(" ")})`);
  root.setProperty("--accent-ink", luminance(accent) > 0.55 ? "#17130a" : "#ffffff");
}

function isOn() {
  return Boolean(state?.pilot?.state);
}

function powerOf(brightness) {
  return isOn() ? Math.max(0.15, brightness / 100) : 0;
}

function applyColors() {
  paint(liveColor(), powerOf(state?.brightness ?? 100));
}

/* ---------- render ---------- */

function render(snapshot) {
  state = snapshot;
  timerSeconds = snapshot.timer_left;
  applyColors();
  syncView();

  const pilot = snapshot.pilot;
  const currentScene = pilot?.scene_id ? sceneById(pilot.scene_id) : null;
  $("deviceName").textContent = snapshot.device_name || "Mi foco";
  $("backendChip").textContent = snapshot.backend_label;
  document.body.classList.toggle("is-off", !pilot?.state);
  document.body.classList.toggle("is-offline", !pilot);
  document.body.classList.toggle("is-animated", Boolean(pilot?.state && currentScene?.animated));

  $("sceneName").textContent = !pilot
    ? snapshot.connected
      ? "El foco no responde"
      : "Sin foco configurado"
    : pilot.state
      ? currentScene
        ? currentScene.name
        : `Blanco · ${pilot.temp ?? snapshot.temperature} K`
      : "Apagado";
  $("powerBtn").disabled = !pilot;
  $("powerBtn").setAttribute("aria-pressed", String(Boolean(pilot?.state)));

  setBrightness(snapshot.brightness);
  const [min, max] = snapshot.white_range ?? [2200, 6500];
  const temperature = $("temperature");
  temperature.min = min;
  temperature.max = max;
  if (document.activeElement !== temperature) temperature.value = snapshot.temperature;
  $("temperatureValue").textContent = `${snapshot.temperature} K`;

  $("stamp").textContent = pilot?.rssi != null ? `${pilot.rssi} dBm` : "";
  $("statusText").textContent = snapshot.busy ? "Trabajando…" : snapshot.status || "Listo";
  $("errorText").textContent = snapshot.error ?? "";
  document.body.classList.toggle("is-busy", Boolean(snapshot.busy));

  renderSettings(snapshot);
  renderScenes(snapshot);
  renderColors(snapshot);
  renderTimer();

  // El foco local dejó de contestar (p. ej. el router le dio otra IP): buscarlo, como mucho cada minuto.
  if (snapshot.backend === "local" && snapshot.has_local && !pilot && Date.now() - lastAutoScan > 60000) {
    lastAutoScan = Date.now();
    scan({ auto: true });
  }
}

let lastAutoScan = 0;

const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");
let shownBrightness = null;
let countFrame = 0;

/* El número cuenta hasta el valor nuevo; mientras se arrastra, sigue al dedo sin retraso. */
function setBrightness(value, { instant = false } = {}) {
  const slider = $("brightness");
  if (document.activeElement !== slider) slider.value = value;
  slider.style.setProperty("--v", value);
  $("lampBtn").setAttribute("aria-valuenow", value);
  $("lampBtn").style.setProperty("--lv", isOn() ? value / 100 : 0);
  const label = $("brightnessValue");
  const on = isOn() || !state?.pilot;
  document.body.classList.toggle("shows-word", !on);
  cancelAnimationFrame(countFrame);
  if (!on) {
    label.textContent = "Apagado";
    shownBrightness = null;
    return;
  }
  const from = shownBrightness ?? 0;
  shownBrightness = value;
  if (instant || reduceMotion.matches || from === value) {
    label.textContent = value;
    return;
  }
  const started = performance.now();
  const duration = Math.min(520, 160 + Math.abs(value - from) * 6);
  const step = (now) => {
    const t = Math.min(1, (now - started) / duration);
    const eased = 1 - Math.pow(1 - t, 4);
    label.textContent = Math.round(from + (value - from) * eased);
    if (t < 1) countFrame = requestAnimationFrame(step);
  };
  countFrame = requestAnimationFrame(step);
}

function renderSettings(snapshot) {
  const keep = (id, value) => {
    if (document.activeElement !== $(id)) $(id).value = value;
  };
  keep("localExport", snapshot.local_export);
  keep("localIp", snapshot.local_ip);
  keep("localMac", snapshot.local_mac);
  keep("cloudDeviceId", snapshot.cloud_device_id);
  keep("cloudHomeId", snapshot.cloud_home_id);
  keep("cloudMac", snapshot.cloud_mac);
  keep("cloudMin", snapshot.cloud_min);
  keep("cloudMax", snapshot.cloud_max);

  for (const button of $("backendSwitch").querySelectorAll(".segmented__btn")) {
    button.classList.toggle("is-active", button.dataset.backend === snapshot.backend);
  }
  $("backendSwitch").dataset.backend = snapshot.backend;
  document.querySelector(".app").dataset.backend = snapshot.backend;

  const chip = (id, done, label) => {
    $(id).textContent = label;
    $(id).classList.toggle("chip--ok", done);
  };
  chip("localState", snapshot.has_local, snapshot.has_local ? "Conectado" : "Sin configurar");
  const cloudLabel = snapshot.has_cloud && snapshot.cloud_session
    ? "Conectado"
    : snapshot.cloud_ready
      ? "Falta iniciar sesión"
      : "Sin credenciales";
  chip("cloudState", snapshot.has_cloud && snapshot.cloud_session, cloudLabel);

  // Pasos de Cloud: cada uno se marca solo cuando ya está hecho.
  const hasDevice = Boolean(snapshot.cloud_device_id && snapshot.cloud_home_id);
  $("stepExport").classList.toggle("is-done", hasDevice);
  $("stepOauth").classList.toggle("is-done", snapshot.cloud_ready);
  $("stepLogin").classList.toggle("is-done", snapshot.cloud_session);
  $("oauthFields").hidden = snapshot.cloud_ready && !editingOauth;
  $("saveOauthBtn").textContent = snapshot.cloud_ready && !editingOauth
    ? "Reemplazar credenciales"
    : "Guardar credenciales";
  $("loginCloudBtn").disabled = !snapshot.cloud_ready || !hasDevice;
  $("clearSessionBtn").hidden = !snapshot.cloud_session;

  const routine = snapshot.routine;
  $("routineEnabled").checked = routine.enabled;
  keep("routineAction", routine.action);
  const hour = Math.floor(routine.minute_of_day / 60);
  const minute = routine.minute_of_day % 60;
  keep("routineTime", `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`);
  $("routineNote").textContent = routine.last_run_day
    ? `Una vez al día, con la hora de este equipo. Última vez: ${routine.last_run_day}.`
    : "Una vez al día, con la hora de este equipo.";
}

function renderColors(snapshot) {
  const row = $("colorRow");
  const colors = snapshot.colors ?? [];
  const signature = colors.map((color) => color.join(",")).join(";");
  if (row.dataset.signature === signature) {
    markColor();
    return;
  }
  row.dataset.signature = signature;
  row.replaceChildren();

  colors.forEach((color, index) => {
    const button = document.createElement("button");
    button.className = "swatch";
    button.style.setProperty("--dot", `rgb(${color.join(" ")})`);
    button.style.setProperty("--i", index);
    button.setAttribute("aria-label", `Color ${toHex(color)}`);
    button.title = toHex(color);
    button.dataset.hex = toHex(color);
    button.addEventListener("click", () => {
      paint(color, powerOf(state?.brightness ?? 100) || 1, null);
      markColor(button.dataset.hex);
      perform("color", color);
    });
    row.append(button);
  });

  const custom = document.createElement("label");
  custom.className = "swatch swatch--custom";
  custom.style.setProperty("--i", colors.length);
  custom.title = "Cualquier color";
  const input = document.createElement("input");
  input.type = "color";
  input.value = toHex(liveColor());
  input.setAttribute("aria-label", "Elegir cualquier color");
  input.addEventListener("input", (event) =>
    paint(hexToRgb(event.target.value), powerOf(state?.brightness ?? 100) || 1, null),
  );
  input.addEventListener("change", (event) => perform("color", hexToRgb(event.target.value)));
  custom.append(input);
  row.append(custom);
  markColor();
}

function markColor(hex = state?.pilot?.color ? toHex(state.pilot.color) : null) {
  for (const swatch of $("colorRow").querySelectorAll(".swatch[data-hex]")) {
    swatch.classList.toggle("is-current", swatch.dataset.hex === hex);
  }
}

/* Las 28 escenas de WiZ en cuatro grupos, para no mostrarlas todas a la vez. */
const SCENE_GROUPS = [
  { name: "Blancos", ids: [12, 11, 13, 15, 16, 14] },
  { name: "Ambiente", ids: [1, 2, 3, 5, 6, 7, 9, 10, 18, 23] },
  { name: "Color", ids: [4, 8, 17, 19, 24, 25, 26] },
  { name: "Temporada", ids: [20, 21, 22, 27, 28] },
];
let sceneTab = null;
let followedScene = null;

function groupOf(id) {
  return SCENE_GROUPS.findIndex((group) => group.ids.includes(id));
}

function renderTabs() {
  const tabs = $("sceneTabs");
  if (!tabs.querySelector(".tab")) {
    SCENE_GROUPS.forEach((group, index) => {
      const tab = document.createElement("button");
      tab.className = "tab";
      tab.setAttribute("role", "tab");
      tab.textContent = group.name;
      tab.addEventListener("click", () => {
        sceneTab = index;
        renderScenes(state);
      });
      tabs.append(tab);
    });
  }
  const all = [...tabs.querySelectorAll(".tab")];
  all.forEach((tab, index) => {
    const active = index === sceneTab;
    tab.classList.toggle("is-active", active);
    tab.setAttribute("aria-selected", String(active));
  });
  // La barra se desliza bajo la pestaña activa, como en Windows 11.
  const active = all[sceneTab];
  const bar = $("tabsBar");
  bar.style.width = `${active.offsetWidth - 24}px`;
  bar.style.transform = `translateX(${active.offsetLeft + 12}px)`;
}

function renderScenes(snapshot) {
  const grid = $("sceneGrid");
  const current = snapshot.pilot?.state ? snapshot.pilot.scene_id : null;
  // La pestaña sigue a la escena del foco cuando cambia; si no, se queda donde la dejó el usuario.
  if (current && current !== followedScene && groupOf(current) >= 0) sceneTab = groupOf(current);
  followedScene = current;
  sceneTab ??= 0;
  renderTabs();
  const byId = new Map(snapshot.scenes.map((scene) => [scene.id, scene]));
  const scenes = SCENE_GROUPS[sceneTab].ids.map((id) => byId.get(id)).filter(Boolean);
  const signature = JSON.stringify([sceneTab, scenes.map((scene) => scene.id)]);
  if (grid.dataset.signature === signature) {
    markScene(current);
    return;
  }
  grid.dataset.signature = signature;
  grid.replaceChildren();

  if (!scenes.length) {
    const empty = document.createElement("p");
    empty.className = "note";
    empty.textContent = "Este foco todavía no tiene escenas.";
    grid.append(empty);
    markScene(null);
    return;
  }
  scenes.forEach((scene, index) => {
    const button = document.createElement("button");
    button.className = "scene";
    button.classList.toggle("is-animated", scene.animated);
    button.title = scene.animated ? `${scene.name}, en movimiento` : scene.name;
    window.Light.palette({ id: scene.id, rgb: scene.color }).forEach((hex, step) =>
      button.style.setProperty(`--c${step}`, hex),
    );
    button.style.setProperty("--i", Math.min(index, 12));
    const name = document.createElement("span");
    name.className = "scene__name";
    name.textContent = scene.name;
    button.append(name);
    button.dataset.id = scene.id;
    button.addEventListener("click", () => {
      paint(scene.color, powerOf(state?.brightness ?? 100) || 1, scene);
      markScene(scene.id);
      run("apply_scene", { id: scene.id });
    });
    grid.append(button);
  });
  requestAnimationFrame(() => markScene(current));
}

/* Un solo marcador viaja entre escenas: se ve de dónde viene y a dónde va la luz. */
function markScene(id) {
  const mark = $("sceneMark");
  let target = null;
  for (const button of $("sceneGrid").querySelectorAll(".scene")) {
    const active = Number(button.dataset.id) === id;
    button.classList.toggle("is-current", active);
    button.setAttribute("aria-pressed", String(active));
    if (active) target = button;
  }
  mark.classList.toggle("is-visible", Boolean(target));
  if (!target) return;
  mark.style.width = `${target.offsetWidth}px`;
  mark.style.height = `${target.offsetHeight}px`;
  mark.style.transform = `translate(${target.offsetLeft}px, ${target.offsetTop}px)`;
}

function renderTimer() {
  const chip = $("timerChip");
  const armed = timerSeconds !== null && timerSeconds !== undefined;
  chip.classList.toggle("chip--ok", armed);
  if (!armed) {
    chip.textContent = "Sin programar";
    return;
  }
  const minutes = Math.floor(timerSeconds / 60);
  const seconds = timerSeconds % 60;
  chip.textContent = `Apaga en ${minutes}:${String(seconds).padStart(2, "0")}`;
}

/* ---------- acciones ---------- */

async function invoke(command, args) {
  if (!bridge) return null;
  try {
    return await bridge.invoke(command, args);
  } catch (error) {
    $("errorText").textContent = String(error);
    return null;
  }
}

/* run() solo para comandos que devuelven un Snapshot: renderizar una lista
   (discover, load_export) deja la interfaz sin estado. */
async function run(command, args) {
  const snapshot = await invoke(command, args);
  if (snapshot) render(snapshot);
  return snapshot;
}

async function perform(kind, value) {
  return run("perform", value === undefined ? { kind } : { kind, value });
}

function on(id, event, handler) {
  $(id).addEventListener(event, handler);
}

/* Ignición: al encender, la luz se abre en círculo desde el punto tocado; al apagar, la sombra la cierra. */
function ignite(on, x, y) {
  const bloom = $("lightBloom");
  const box = light.getBoundingClientRect();
  const ox = x ?? box.width / 2;
  const oy = y ?? box.height / 2;
  light.classList.add("is-igniting");
  clearTimeout(light.igniteTimer);
  light.igniteTimer = setTimeout(() => light.classList.remove("is-igniting"), 820);
  // Con WebGPU el shader dibuja el frente de luz; al apagar basta con que la potencia baje.
  if (window.Light.gpu) {
    if (on) window.Light.ignite((box.left + ox) / innerWidth, (box.top + oy) / innerHeight);
    return;
  }
  const radius = Math.hypot(Math.max(ox, box.width - ox), Math.max(oy, box.height - oy));
  bloom.classList.toggle("is-dark", !on);
  bloom.getAnimations().forEach((animation) => animation.cancel());
  if (reduceMotion.matches) {
    bloom.animate([{ opacity: 0.6 }, { opacity: 0 }], { duration: 260, easing: "ease-out" });
    return;
  }
  const circle = (r) => `circle(${r}px at ${ox}px ${oy}px)`;
  bloom.animate(
    [
      // Cada tramo lleva su curva: el círculo se abre con decisión y luego la luz se asienta.
      { clipPath: circle(0), opacity: 1, easing: "cubic-bezier(0.16, 1, 0.3, 1)" },
      { clipPath: circle(radius), opacity: 1, offset: 0.55, easing: "cubic-bezier(0.4, 0, 0.2, 1)" },
      { clipPath: circle(radius), opacity: 0 },
    ],
    { duration: on ? 900 : 600 },
  );
}

/* Encender/apagar se ve al instante; el foco confirma después y render corrige si falla. */
function togglePower(x, y) {
  if (!state?.pilot) return;
  const next = !state.pilot.state;
  state.pilot.state = next;
  ignite(next, x, y);
  document.body.classList.toggle("is-off", !next);
  applyColors();
  setBrightness(state.brightness);
  $("sceneName").textContent = next ? "Encendiendo…" : "Apagado";
  markScene(next ? state.pilot.scene_id : null);
  perform(next ? "on" : "off");
}

/* ---------- vistas ---------- */

function syncView() {
  const wanted = view ?? (state?.connected ? "home" : "settings");
  document.querySelector(".app").dataset.panel = wanted;
  document.body.dataset.panel = wanted;
  window.Light.active(wanted === "home");
  $("settingsBtn").setAttribute("aria-pressed", String(wanted === "settings"));
}

/* Ajustes entra desde la derecha y la luz vuelve desde la izquierda: el orden espacial se mantiene. */
function goTo(next) {
  if (document.querySelector(".app").dataset.panel === next) return;
  const swap = () => {
    view = next;
    syncView();
  };
  document.documentElement.dataset.towards = next;
  if (document.startViewTransition && !reduceMotion.matches) document.startViewTransition(swap);
  else swap();
}

on("settingsBtn", "click", () =>
  goTo(document.querySelector(".app").dataset.panel === "settings" ? "home" : "settings"),
);

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && document.querySelector(".app").dataset.panel === "settings") goTo("home");
});

on("refreshBtn", "click", () => run("refresh"));
on("powerBtn", "click", (event) => {
  event.stopPropagation();
  // La luz se abre desde el botón, aunque esté en la barra y fuera del panel.
  const button = event.currentTarget.getBoundingClientRect();
  const box = light.getBoundingClientRect();
  togglePower(button.left + button.width / 2 - box.left, button.top + button.height / 2 - box.top);
});

/* ---------- la luz: arrastrar atenúa, un clic enciende o apaga ---------- */

const light = $("lampBtn");
let drag = null;

light.addEventListener("pointerdown", (event) => {
  if (!state?.pilot) return;
  drag = { x: event.clientX, start: state.brightness ?? 100, value: null, ox: event.offsetX, oy: event.offsetY };
  light.setPointerCapture(event.pointerId);
});

light.addEventListener("pointermove", (event) => {
  if (!drag) return;
  const dx = event.clientX - drag.x;
  if (drag.value === null && Math.abs(dx) < 6) return;
  if (!isOn()) return;
  const value = Math.round(Math.max(1, Math.min(100, drag.start + (dx / light.clientWidth) * 140)));
  drag.value = value;
  light.classList.add("is-dragging");
  setBrightness(value, { instant: true });
  paint(liveColor(), powerOf(value));
});

light.addEventListener("pointerup", () => {
  if (!drag) return;
  const { value, ox, oy } = drag;
  drag = null;
  light.classList.remove("is-dragging");
  if (value === null) togglePower(ox, oy);
  else {
    state.brightness = value;
    perform("brightness", value);
  }
});

light.addEventListener("pointercancel", () => {
  drag = null;
  light.classList.remove("is-dragging");
  applyColors();
});

light.addEventListener("keydown", (event) => {
  if (!state?.pilot) return;
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    togglePower();
    return;
  }
  const step = { ArrowRight: 5, ArrowUp: 5, ArrowLeft: -5, ArrowDown: -5 }[event.key];
  if (!step || !isOn()) return;
  event.preventDefault();
  const value = Math.max(1, Math.min(100, (state.brightness ?? 100) + step));
  state.brightness = value;
  setBrightness(value, { instant: true });
  applyColors();
  clearTimeout(light.sendTimer);
  light.sendTimer = setTimeout(() => perform("brightness", value), 250);
});

/* ---------- diales ---------- */

on("brightness", "input", (event) => {
  const value = Number(event.target.value);
  setBrightness(value, { instant: true });
  paint(liveColor(), powerOf(value));
});

on("brightness", "change", (event) => {
  state.brightness = Number(event.target.value);
  perform("brightness", state.brightness);
});

on("temperature", "input", (event) => {
  const value = Number(event.target.value);
  $("temperatureValue").textContent = `${value} K`;
  paint(kelvinToRgb(value), powerOf(state?.brightness ?? 100) || 1, null);
});

on("temperature", "change", (event) => perform("temperature", Number(event.target.value)));


/* ---------- rutinas ---------- */

on("timerPresets", "click", (event) => {
  const preset = event.target.closest(".preset");
  if (!preset) return;
  $("timerMinutes").value = preset.dataset.minutes;
  run("start_timer", { minutes: Number(preset.dataset.minutes) });
});

on("timerStartBtn", "click", () => {
  const minutes = Math.max(1, Math.min(600, Number($("timerMinutes").value) || 1));
  run("start_timer", { minutes });
});

on("timerCancelBtn", "click", () => run("cancel_timer"));

on("routineSaveBtn", "click", () => {
  const [hour, minute] = $("routineTime").value.split(":").map(Number);
  run("set_routine", {
    enabled: $("routineEnabled").checked,
    minuteOfDay: (hour || 0) * 60 + (minute || 0),
    action: $("routineAction").value,
  });
});

/* ---------- conexión local: descubrimiento automático ---------- */

function useLight(found) {
  return run("save_local_setup", {
    exportPath: $("localExport").value,
    ip: found.ip,
    mac: found.mac,
    deviceName: $("deviceNameInput").value,
  });
}

function showFound(lights) {
  const list = $("foundList");
  list.replaceChildren();
  const current = (state?.local_mac ?? "").replace(/[:-]/g, "").toLowerCase();
  for (const found of lights) {
    const item = document.createElement("li");
    const label = document.createElement("span");
    label.className = "found__label";
    label.textContent = `${found.ip}`;
    const meta = document.createElement("span");
    meta.className = "note";
    meta.textContent = `${found.mac} · ${found.rssi ?? "?"} dBm`;
    label.append(meta);
    const isCurrent = found.mac === current && state?.local_ip === found.ip;
    const pick = document.createElement("button");
    pick.className = isCurrent ? "btn btn--quiet" : "btn btn--primary";
    pick.textContent = isCurrent ? "En uso" : "Usar";
    pick.disabled = isCurrent;
    pick.addEventListener("click", () => useLight(found));
    item.append(label, pick);
    list.append(item);
  }
}

let scanning = null;

/* Busca focos y, si puede decidir solo, los configura:
   - sin foco local y uno solo en la red: lo usa;
   - el foco guardado cambió de IP (DHCP): la actualiza. */
function scan({ auto = false } = {}) {
  if (!bridge) return Promise.resolve();
  if (scanning) return scanning;
  $("discoveryNote").textContent = "Buscando focos WiZ en tu red…";
  $("discoverBtn").disabled = true;
  scanning = invoke("discover")
    .then(async (lights) => {
      if (!lights) return;
      showFound(lights);
      $("discoveryNote").textContent = lights.length
        ? `${lights.length === 1 ? "1 foco encontrado" : `${lights.length} focos encontrados`} en tu red.`
        : "No encontré focos WiZ. Comprueba que estén encendidos y en esta red Wi-Fi.";
      if (!auto || !state) return;
      const saved = (state.local_mac ?? "").replace(/[:-]/g, "").toLowerCase();
      const same = lights.find((found) => found.mac === saved);
      if (state.has_local && same && same.ip !== state.local_ip) {
        await useLight(same);
        $("statusText").textContent = `El foco cambió de IP: ahora ${same.ip}`;
      } else if (!state.has_local && lights.length === 1) {
        await useLight(lights[0]);
        view = "home";
        syncView();
      }
    })
    .finally(() => {
      scanning = null;
      $("discoverBtn").disabled = false;
    });
  return scanning;
}

on("discoverBtn", "click", () => scan());

on("saveLocalBtn", "click", () =>
  run("save_local_setup", {
    exportPath: $("localExport").value,
    ip: $("localIp").value,
    mac: $("localMac").value,
    deviceName: $("deviceNameInput").value,
  }),
);

on("backendSwitch", "click", (event) => {
  const button = event.target.closest(".segmented__btn");
  if (button) run("set_backend", { backend: button.dataset.backend });
});

/* ---------- WiZ Cloud en tres pasos ---------- */

let editingOauth = false;

/* El export trae device_id, home_id, MAC y rango: con él no hay que copiar nada a mano. */
on("pickExportBtn", "click", async () => {
  const path = await invoke("pick_export");
  if (!path) return;
  $("localExport").value = path;
  const devices = await invoke("load_export", { path });
  if (!devices?.length) return;
  const saved = (state?.local_mac ?? "").replace(/[:-]/g, "").toLowerCase();
  const device = devices.find((candidate) => candidate.mac === saved) ?? devices[0];
  if (!state?.local_mac) $("deviceNameInput").value = device.name;
  if (device.device_id && device.home_id) {
    await run("save_cloud_setup", {
      deviceId: String(device.device_id),
      homeId: String(device.home_id),
      mac: device.mac,
      minimum: device.white_range ? String(device.white_range[0]) : "",
      maximum: device.white_range ? String(device.white_range[1]) : "",
    });
  }
  // Si el foco local ya funciona, el export solo añade la clave para firmar.
  if (state?.has_local) {
    await run("save_local_setup", {
      exportPath: path,
      ip: state.local_ip,
      mac: state.local_mac,
      deviceName: "",
    });
  }
});

on("saveOauthBtn", "click", async () => {
  if (state?.cloud_ready && !editingOauth) {
    editingOauth = true;
    renderSettings(state);
    $("googleClientId").focus();
    return;
  }
  const saved = await run("save_cloud_oauth", {
    googleClientId: $("googleClientId").value,
    googleClientSecret: $("googleClientSecret").value,
    wizClientId: $("wizClientId").value,
    wizClientSecret: $("wizClientSecret").value,
  });
  if (saved) {
    editingOauth = false;
    // Los secretos no se quedan en el formulario después de guardarlos.
    for (const id of ["googleClientId", "googleClientSecret", "wizClientId", "wizClientSecret"]) {
      $(id).value = "";
    }
    renderSettings(saved);
  }
});

on("loginCloudBtn", "click", async () => {
  if (state?.backend !== "cloud" && !(await run("set_backend", { backend: "cloud" }))) return;
  $("statusText").textContent = "Termina el inicio de sesión en tu navegador…";
  perform("connect");
});

on("clearSessionBtn", "click", () => run("clear_cloud_session_command"));

on("saveCloudBtn", "click", () =>
  run("save_cloud_setup", {
    deviceId: $("cloudDeviceId").value,
    homeId: $("cloudHomeId").value,
    mac: $("cloudMac").value,
    minimum: $("cloudMin").value,
    maximum: $("cloudMax").value,
  }),
);

/* ---------- arranque ---------- */

/* La primera lectura enciende la ventana con la misma ignición que el botón. */
function boot() {
  new ResizeObserver(() => markScene(state?.pilot?.state ? state.pilot.scene_id : null)).observe($("sceneGrid"));
  if (isOn()) ignite(true);
}

async function start() {
  if (bridge) {
    render(await bridge.invoke("snapshot"));
    // Las pestañas necesitan el catálogo completo de escenas.
    if (state.scenes.length < 28) await run("set_show_all_scenes", { showAll: true });
    boot();
    // Tauri v2 entrega {event, id, payload}; el sondeo manda el Snapshot dentro.
    await bridge.listen("state", (event) => render(event?.payload ?? event));
    // Al abrir: busca focos si no hay ninguno o si el guardado no contesta.
    if (state.backend === "local" && !state.has_local) scan({ auto: true });
    return;
  }
  demo();
  boot();
}

/* Modo navegador: sirve para revisar el diseño fuera de la app.
   `animated` replica state::scene_is_animated: los colores de WiZ se mueven. */
const CATALOG = [
  [12, "Luz de día", [232, 172, 52], false],
  [11, "Blanco cálido", [232, 156, 74], false],
  [13, "Blanco frío", [116, 166, 204], false],
  [15, "Enfoque", [82, 156, 130], false],
  [16, "Relajarse", [140, 118, 198], false],
  [14, "Luz nocturna", [74, 92, 168], false],
  [1, "Océano", [52, 138, 189], true],
  [2, "Romance", [224, 96, 132], true],
  [3, "Atardecer", [240, 133, 69], true],
  [4, "Fiesta", [198, 84, 206], true],
  [5, "Chimenea", [214, 105, 48], true],
  [6, "Acogedor", [196, 148, 96], true],
  [7, "Bosque", [72, 150, 91], true],
  [8, "Colores pastel", [210, 156, 196], true],
  [9, "Despertar", [243, 188, 92], true],
  [10, "Hora de dormir", [70, 82, 140], true],
  [17, "Colores reales", [96, 165, 196], true],
  [18, "Hora de TV", [104, 118, 168], true],
  [19, "Crecimiento de plantas", [98, 168, 108], true],
  [20, "Primavera", [150, 196, 90], true],
  [21, "Verano", [240, 178, 56], true],
  [22, "Otoño", [196, 120, 60], true],
  [23, "Inmersión", [42, 108, 160], true],
  [24, "Jungla", [48, 140, 96], true],
  [25, "Mojito", [120, 200, 140], true],
  [26, "Club", [170, 70, 200], true],
  [27, "Navidad", [200, 60, 70], true],
  [28, "Halloween", [240, 130, 40], true],
].map(([id, name, color, animated]) => ({ id, name, color, animated }));

function demo() {
  const snapshot = {
    backend: "local",
    backend_label: "Casa por Wi-Fi",
    device_name: "Luz de Dormitorio",
    connected: true,
    has_local: true,
    has_cloud: false,
    cloud_session: false,
    cloud_ready: false,
    pilot: { state: true, scene_id: 12, temp: 4200, dimming: 100, color: null, rssi: -59 },
    brightness: 100,
    temperature: 4200,
    color: [232, 172, 52],
    white_range: [2700, 6500],
    favorites: CATALOG.slice(0, 6),
    scenes: CATALOG,
    timer_left: null,
    colors: [
      [52, 138, 189],
      [224, 96, 132],
      [240, 133, 69],
      [198, 84, 206],
      [214, 105, 48],
      [72, 150, 91],
      [210, 156, 196],
      [200, 60, 70],
    ],
    status: "Listo",
    error: null,
    busy: false,
    routine: { enabled: true, minute_of_day: 1320, action: "off", last_run_day: null },
    local_ip: "192.168.100.9",
    local_mac: "a4:cf:e9:2b:71:08",
    local_export: "wiz-home.json",
    cloud_device_id: "",
    cloud_home_id: "",
    cloud_mac: "",
    cloud_min: "2700",
    cloud_max: "6500",
  };
  render(snapshot);

  // El foquito va cambiando de color como lo haría con la rutina.
  const rotation = CATALOG;
  let step = 0;
  setInterval(() => {
    step = (step + 1) % rotation.length;
    const next = rotation[step];
    snapshot.pilot.scene_id = next.id;
    snapshot.pilot.color = null;
    snapshot.pilot.temp = next.id === 13 ? 6500 : next.id === 11 ? 2700 : 4200;
    snapshot.brightness = 60 + ((step * 9) % 41);
    snapshot.pilot.dimming = snapshot.brightness;
    render(snapshot);
  }, 6000);
}

setInterval(() => {
  if (timerSeconds === null || timerSeconds === undefined) return;
  timerSeconds = Math.max(0, timerSeconds - 1);
  renderTimer();
}, 1000);

start();
