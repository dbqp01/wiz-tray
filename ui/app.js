/* Interfaz de WiZ Tray: estado en vivo + foquito que sigue a la luz. */

const bridge = window.__TAURI__
  ? {
      invoke: (command, args) => window.__TAURI__.core.invoke(command, args),
      listen: (event, handler) =>
        window.__TAURI__.event.listen(event, (payload) => handler(payload.payload)),
    }
  : null;

const $ = (id) => document.getElementById(id);
const OFF_RGB = [74, 74, 84];
const AMBER = [245, 181, 68];

let state = null;
let timerSeconds = null;
let lastRead = null;
let lastLiveHex = null;

/* Parallax: la lámpara y la atmósfera siguen al puntero. */
let parallaxFrame = 0;
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");
document.addEventListener(
  "pointermove",
  (event) => {
    if (reduceMotion.matches || parallaxFrame) return;
    parallaxFrame = requestAnimationFrame(() => {
      const root = document.documentElement.style;
      root.setProperty("--mx", ((event.clientX / innerWidth) * 2 - 1).toFixed(3));
      root.setProperty("--my", ((event.clientY / innerHeight) * 2 - 1).toFixed(3));
      parallaxFrame = 0;
    });
  },
  { passive: true },
);
document.addEventListener("pointerleave", () => {
  document.documentElement.style.setProperty("--mx", "0");
  document.documentElement.style.setProperty("--my", "0");
});

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
  return [...(state?.scenes ?? []), ...(state?.favorites ?? [])].find(
    (candidate) => candidate.id === sceneId,
  );
}

function sceneColor(sceneId) {
  return sceneById(sceneId)?.color ?? null;
}

function liveColor() {
  const pilot = state?.pilot;
  if (!pilot) return AMBER;
  if (pilot.color) return pilot.color;
  const scene = sceneColor(pilot.scene_id);
  if (scene) return scene;
  return kelvinToRgb(pilot.temp ?? state?.temperature ?? 4000);
}

function luminance([r, g, b]) {
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

function toHex([r, g, b]) {
  return `#${[r, g, b].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function applyColors() {
  const pilot = state?.pilot;
  const on = Boolean(pilot?.state);
  const power = on ? Math.max(0.15, (state?.brightness ?? 100) / 100) : 0;
  const live = liveColor();
  const bulb = on ? live : OFF_RGB;

  const root = document.documentElement;
  root.style.setProperty("--bulb", `rgb(${bulb.join(" ")})`);
  root.style.setProperty("--bulb-hi", `rgb(${tint(bulb, 0.72).join(" ")})`);
  root.style.setProperty("--power", power.toFixed(2));

  const accent = on ? lift(live) : AMBER;
  root.style.setProperty("--accent", `rgb(${accent.join(" ")})`);
  root.style.setProperty("--accent-ink", luminance(accent) > 0.55 ? "#17130a" : "#fdf6ea");

  // Cada cambio de color hace latir el foquito.
  const hex = toHex(bulb);
  if (lastLiveHex && lastLiveHex !== hex) flashLamp();
  lastLiveHex = hex;
}

function flashLamp() {
  const lamp = document.querySelector(".bulb");
  if (!lamp) return;
  lamp.classList.remove("is-flash");
  void lamp.offsetWidth;
  lamp.classList.add("is-flash");
}

function lift([r, g, b]) {
  return [r, g, b].map((value) => Math.round(Math.min(255, value + (255 - value) * 0.32)));
}

function tint(rgb, amount) {
  return rgb.map((value) => Math.round(value + (255 - value) * amount));
}

/* ---------- render ---------- */

function render(snapshot) {
  state = snapshot;
  timerSeconds = snapshot.timer_left;

  applyColors();
  syncTab(snapshot);

  const pilot = snapshot.pilot;
  const currentScene = pilot?.scene_id ? sceneById(pilot.scene_id) : null;
  const sceneName = currentScene?.name ?? "Modo actual";

  // Las escenas animadas de WiZ hacen girar el arco del foquito.
  document
    .querySelector(".bulb")
    ?.classList.toggle("is-animated", Boolean(pilot?.state && currentScene?.animated));

  $("deviceName").textContent = snapshot.device_name || "Mi foco";
  $("backendChip").textContent = snapshot.backend_label;

  $("sceneName").textContent = !pilot
    ? "Sin conexión con el foco"
    : pilot.state
      ? `${sceneName} · ${pilot.temp ?? snapshot.temperature} K · ${snapshot.brightness} %`
      : "Foco apagado";

  $("powerBtn").textContent = pilot?.state ? "Apagar" : "Encender";
  $("powerBtn").disabled = !pilot;

  lastRead = new Date();
  $("stamp").textContent = `Actualizado ${lastRead.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  })}`;

  setSlider("brightness", snapshot.brightness, 1, 100);
  $("brightnessValue").textContent = `${snapshot.brightness} %`;
  document.documentElement.style.setProperty("--v", snapshot.brightness);

  const [min, max] = snapshot.white_range ?? [2200, 6500];
  const temperature = $("temperature");
  temperature.min = min;
  temperature.max = max;
  setSlider("temperature", snapshot.temperature, min, max);
  $("temperatureValue").textContent = `${snapshot.temperature} K`;
  $("tempMin").textContent = `${min} K`;
  $("tempMax").textContent = `${max} K`;

  $("statColorHex").textContent = toHex(liveColor());
  $("statSignal").textContent =
    pilot?.rssi !== null && pilot?.rssi !== undefined ? `${pilot.rssi} dBm` : "—";
  $("statRange").textContent = `${min}–${max} K`;

  $("localState").textContent = snapshot.has_local ? "Configurado" : "Sin configurar";
  $("cloudState").textContent = snapshot.cloud_ready ? "Credenciales guardadas" : "Sin credenciales";
  $("statusText").textContent = snapshot.busy ? "Trabajando…" : snapshot.status || "Listo";
  $("errorText").textContent = snapshot.error ?? "";
  document.body.classList.toggle("is-busy", Boolean(snapshot.busy));

  for (const button of $("backendSwitch").querySelectorAll(".segmented__btn")) {
    button.classList.toggle("is-active", button.dataset.backend === snapshot.backend);
  }

  if (document.activeElement !== $("localExport")) $("localExport").value = snapshot.local_export;
  if (document.activeElement !== $("localIp")) $("localIp").value = snapshot.local_ip;
  if (document.activeElement !== $("localMac")) $("localMac").value = snapshot.local_mac;
  if (document.activeElement !== $("cloudDeviceId")) $("cloudDeviceId").value = snapshot.cloud_device_id;
  if (document.activeElement !== $("cloudHomeId")) $("cloudHomeId").value = snapshot.cloud_home_id;
  if (document.activeElement !== $("cloudMac")) $("cloudMac").value = snapshot.cloud_mac;
  if (document.activeElement !== $("cloudMin")) $("cloudMin").value = snapshot.cloud_min;
  if (document.activeElement !== $("cloudMax")) $("cloudMax").value = snapshot.cloud_max;

  const routine = snapshot.routine;
  $("routineEnabled").checked = routine.enabled;
  $("routineAction").value = routine.action;
  const hour = Math.floor(routine.minute_of_day / 60);
  const minute = routine.minute_of_day % 60;
  $("routineTime").value = `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
  $("routineNote").textContent = routine.last_run_day
    ? `Se aplica una vez al día, a la hora local de este equipo. Última ejecución: ${routine.last_run_day}.`
    : "Se aplica una vez al día, a la hora local de este equipo.";

  $("allScenesBtn").textContent =
    snapshot.scenes.length > snapshot.favorites.length ? "Ver solo las favoritas" : "Ver las 28 escenas";

  renderScenes(snapshot);
  renderColors(snapshot);
  renderTimer();
}

function renderColors(snapshot) {
  const row = $("colorRow");
  const colors = snapshot.colors ?? [];
  const signature = colors.map((color) => color.join(",")).join(";");
  if (row.dataset.signature === signature) return;
  row.dataset.signature = signature;
  row.replaceChildren();
  colors.forEach((color, index) => {
    const button = document.createElement("button");
    button.className = "swatch";
    button.style.setProperty("--dot", `rgb(${color.join(" ")})`);
    button.style.setProperty("--i", index);
    button.style.setProperty(
      "--angle",
      `${colors.length > 1 ? -78 + (156 / (colors.length - 1)) * index : 0}deg`,
    );
    button.title = `Color ${color.join(", ")}`;
    button.setAttribute("aria-label", `Aplicar color ${toHex(color)}`);
    button.addEventListener("click", () => {
      if (bridge) perform("color", color);
    });
    row.append(button);
  });
}

function setSlider(id, value, min, max) {
  const slider = $(id);
  if (document.activeElement !== slider) slider.value = value;
}

function renderScenes(snapshot) {
  const grid = $("sceneGrid");
  const current = snapshot.pilot?.scene_id;
  const signature = JSON.stringify([
    snapshot.scenes.map((scene) => scene.id),
    snapshot.favorites.map((scene) => scene.id),
    current,
  ]);
  if (grid.dataset.signature === signature) return;
  grid.dataset.signature = signature;
  grid.replaceChildren();

  if (!snapshot.scenes.length) {
    const empty = document.createElement("p");
    empty.className = "note";
    empty.textContent = "Este foco todavía no tiene escenas cargadas.";
    grid.append(empty);
  } else {
    for (const scene of snapshot.scenes) {
      grid.append(sceneTile(scene, scene.id === current));
    }
  }

  const quick = $("quickSceneGrid");
  quick.replaceChildren();
  const favourites = (snapshot.favorites ?? []).slice(0, 6);
  if (!favourites.length) {
    const empty = document.createElement("p");
    empty.className = "note";
    empty.textContent = "Este foco todavía no tiene escenas marcadas como favoritas.";
    quick.append(empty);
  } else {
    for (const scene of favourites) {
      quick.append(sceneTile(scene, scene.id === current));
    }
  }
}

function sceneTile(scene, isCurrent) {
  const button = document.createElement("button");
  button.className = "scene";
  button.classList.toggle("is-current", isCurrent);
  button.style.setProperty("--dot", `rgb(${scene.color.join(" ")})`);

  const block = document.createElement("span");
  block.className = "scene__block";

  const name = document.createElement("span");
  name.className = "scene__name";
  if (scene.animated) {
    const live = document.createElement("span");
    live.className = "scene__live";
    name.append(live);
  }
  name.append(scene.name);

  button.append(block, name);
  button.addEventListener("click", () => {
    if (bridge) run("apply_scene", { id: scene.id });
  });
  return button;
}

function renderTimer() {
  const chip = $("timerChip");
  const armed = timerSeconds !== null && timerSeconds !== undefined;
  chip.classList.toggle("chip--live", armed);
  if (!armed) {
    chip.textContent = "Sin programar";
    return;
  }
  const minutes = Math.floor(timerSeconds / 60);
  const seconds = timerSeconds % 60;
  chip.textContent = `Apaga en ${minutes}:${String(seconds).padStart(2, "0")}`;
}

/* ---------- acciones ---------- */

async function run(command, args) {
  if (!bridge) return null;
  try {
    const snapshot = await bridge.invoke(command, args);
    if (snapshot) render(snapshot);
    return snapshot;
  } catch (error) {
    $("errorText").textContent = String(error);
    return null;
  }
}

async function perform(kind, value) {
  return run("perform", value === undefined ? { kind } : { kind, value });
}

function on(id, event, handler) {
  $(id).addEventListener(event, handler);
}

/* ---------- pestañas ---------- */

let chosenTab = null;

function syncTab(snapshot) {
  const wanted = chosenTab ?? (snapshot.connected ? "home" : "connections");
  const tabs = [...$("tabs").querySelectorAll(".tab")];
  document.querySelector(".app").dataset.panel = wanted;
  tabs.forEach((item) => {
    const active = item.dataset.tab === wanted;
    item.classList.toggle("is-active", active);
    item.setAttribute("aria-selected", String(active));
  });
  const index = tabs.findIndex((item) => item.dataset.tab === wanted);
  $("tabs").style.setProperty("--tab-index", Math.max(0, index));
  for (const panel of $("panels").querySelectorAll(".panel")) {
    panel.classList.toggle("is-active", panel.dataset.panel === wanted);
  }
}

on("tabs", "click", (event) => {
  const tab = event.target.closest(".tab");
  if (!tab) return;
  chosenTab = tab.dataset.tab;
  syncTab(state ?? {});
});

/* ---------- hero ---------- */

on("powerBtn", "click", () => perform(state?.pilot?.state ? "off" : "on"));
on("daylightBtn", "click", () => perform("daylight"));
on("refreshBtn", "click", () => run("refresh"));
on("lampBtn", "click", () => {
  if (state?.pilot) perform(state.pilot.state ? "off" : "on");
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && chosenTab && chosenTab !== "home") {
    chosenTab = "home";
    syncTab(state ?? {});
  }
});

/* ---------- diales ---------- */

on("brightness", "input", (event) => {
  const value = Number(event.target.value);
  $("brightnessValue").textContent = `${value} %`;
  document.documentElement.style.setProperty("--v", value);
  document.documentElement.style.setProperty("--power", Math.max(0.15, value / 100).toFixed(2));
});

on("brightness", "change", (event) => perform("brightness", Number(event.target.value)));

on("temperature", "input", (event) => {
  const value = Number(event.target.value);
  const rgb = kelvinToRgb(value);
  const live = lift(rgb);
  const root = document.documentElement.style;
  $("temperatureValue").textContent = `${value} K`;
  $("statColorHex").textContent = toHex(rgb);
  root.setProperty("--bulb", `rgb(${rgb.join(" ")})`);
  root.setProperty("--bulb-hi", `rgb(${tint(rgb, 0.72).join(" ")})`);
  root.setProperty("--accent", `rgb(${live.join(" ")})`);
  root.setProperty("--accent-ink", luminance(live) > 0.55 ? "#17130a" : "#fdf6ea");
});

on("temperature", "change", (event) => perform("temperature", Number(event.target.value)));

/* ---------- escenas ---------- */

on("allScenesBtn", "click", () => {
  const showingAll = state && state.scenes.length > state.favorites.length;
  run("set_show_all_scenes", { showAll: !showingAll });
});

on("quickScenesBtn", "click", () => {
  chosenTab = "scenes";
  syncTab(state ?? {});
});

/* ---------- rutinas ---------- */

on("timerStartBtn", "click", () => {
  const minutes = Math.max(1, Number($("timerMinutes").value) || 1);
  run("start_timer", { minutes });
});

on("timerPresets", "click", (event) => {
  const preset = event.target.closest(".preset");
  if (preset) $("timerMinutes").value = preset.dataset.minutes;
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

/* ---------- conexiones ---------- */

on("backendSwitch", "click", (event) => {
  const button = event.target.closest(".segmented__btn");
  if (button) run("set_backend", { backend: button.dataset.backend });
});

on("pickExportBtn", "click", async () => {
  const path = await run("pick_export");
  if (path) {
    $("localExport").value = path;
    const devices = await run("load_export", { path });
    if (devices?.length) {
      const device = devices[0];
      $("localMac").value = device.mac;
      $("deviceNameInput").value = device.name;
      if (device.white_range) {
        $("cloudMin").value = device.white_range[0];
        $("cloudMax").value = device.white_range[1];
      }
      $("discoveryNote").textContent = `${devices.length} foco(s) en el export.`;
    }
  }
});

on("discoverBtn", "click", async () => {
  $("discoveryNote").textContent = "Buscando focos WiZ en tu red…";
  const lights = await run("discover");
  const list = $("foundList");
  list.replaceChildren();
  if (!lights) return;
  $("discoveryNote").textContent = lights.length
    ? `${lights.length} foco(s) encontrados.`
    : "No se encontró ningún foco WiZ en la red.";
  for (const light of lights) {
    const item = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = `${light.mac} · ${light.ip} · ${light.rssi ?? "?"} dBm`;
    const pick = document.createElement("button");
    pick.textContent = "Usar este foco";
    pick.addEventListener("click", () => {
      $("localIp").value = light.ip;
      $("localMac").value = light.mac;
    });
    item.append(label, pick);
    list.append(item);
  }
});

on("saveLocalBtn", "click", () =>
  run("save_local_setup", {
    exportPath: $("localExport").value,
    ip: $("localIp").value,
    mac: $("localMac").value,
    deviceName: $("deviceNameInput").value,
  }),
);

on("saveCloudBtn", "click", () =>
  run("save_cloud_setup", {
    deviceId: $("cloudDeviceId").value,
    homeId: $("cloudHomeId").value,
    mac: $("cloudMac").value,
    minimum: $("cloudMin").value,
    maximum: $("cloudMax").value,
  }),
);

on("saveOauthBtn", "click", () =>
  run("save_cloud_oauth", {
    googleClientId: $("googleClientId").value,
    googleClientSecret: $("googleClientSecret").value,
    wizClientId: $("wizClientId").value,
    wizClientSecret: $("wizClientSecret").value,
  }),
);

on("clearSessionBtn", "click", () => run("clear_cloud_session_command"));

on("loginCloudBtn", "click", () => perform("connect"));

/* ---------- arranque ---------- */

async function start() {
  if (bridge) {
    const snapshot = await bridge.invoke("snapshot");
    render(snapshot);
    await bridge.listen("state", render);
    return;
  }
  demo();
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
