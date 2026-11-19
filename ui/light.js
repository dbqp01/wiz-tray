/* La luz de la ventana: un shader WebGPU pinta cada escena con su paleta y su bucle.
   Sin WebGPU cae a un degradado CSS. Solo dibuja mientras algo se mueve, a 30 fps como mucho. */

(() => {
  /* Paleta sombra / cuerpo / luz y bucle de cada escena de WiZ (id del foco).
     Bucles: 0 quieta · 1 oleaje · 2 ocaso · 3 latido · 4 brasa · 5 follaje · 6 fiesta · 7 amanecer.
     El periodo (s) es lo que tarda en cerrar el bucle sin corte. */
  const LOOKS = {
    12: { pal: ["#b88a4a", "#ffe2b0", "#fff8ec"], mode: 0 }, // Luz de día
    11: { pal: ["#7a3d12", "#ffb46b", "#ffe7c9"], mode: 0 }, // Blanco cálido
    13: { pal: ["#3a4f7a", "#c9d8ff", "#f4f7ff"], mode: 0 }, // Blanco frío
    15: { pal: ["#4c5b6b", "#e3ecff", "#ffffff"], mode: 0 }, // Enfoque
    16: { pal: ["#2b2350", "#8b6fe0", "#f0c4dc"], mode: 0 }, // Relajarse
    14: { pal: ["#070b22", "#24357f", "#6e7fd6"], mode: 0 }, // Luz nocturna
    1: { pal: ["#0b3b6e", "#1e7fcb", "#5fd3e6"], mode: 1, period: 18 }, // Océano
    2: { pal: ["#4a1030", "#d9467a", "#ff9eb3"], mode: 3, period: 5 }, // Romance
    3: { pal: ["#3a1c4f", "#e2574c", "#ffb765"], mode: 2, period: 24 }, // Atardecer
    4: { pal: ["#6a2bd9", "#ff4fa3", "#2ec5ff"], mode: 6, period: 9 }, // Fiesta
    5: { pal: ["#2a0e05", "#d9481c", "#ffb347"], mode: 4, period: 1 }, // Chimenea
    6: { pal: ["#3b1d0c", "#d98a3d", "#ffd59a"], mode: 4, period: 1 }, // Acogedor
    7: { pal: ["#0e2a1c", "#2e8b57", "#c4e38a"], mode: 5, period: 20 }, // Bosque
    8: { pal: ["#9c8ad8", "#f4b6d2", "#bde8e0"], mode: 6, period: 14 }, // Colores pastel
    9: { pal: ["#5a2a1a", "#ff9a5a", "#ffe3b0"], mode: 7, period: 30 }, // Despertar
    10: { pal: ["#1a1238", "#a0522d", "#ffb07a"], mode: 2, period: 30 }, // Hora de dormir
    17: { pal: ["#2a5d8f", "#62c3a1", "#f2d16b"], mode: 6, period: 12 }, // Colores reales
    18: { pal: ["#14183a", "#4e5fa8", "#9fb2e8"], mode: 1, period: 30 }, // Hora de TV
    19: { pal: ["#3b0f4f", "#c44fd6", "#ff6a9a"], mode: 5, period: 24 }, // Crecimiento de plantas
    20: { pal: ["#2e5a1e", "#9fd36a", "#ffd6e7"], mode: 5, period: 16 }, // Primavera
    21: { pal: ["#7a3b00", "#ffb020", "#fff0a8"], mode: 2, period: 20 }, // Verano
    22: { pal: ["#3a1606", "#c4642a", "#f2b25c"], mode: 5, period: 22 }, // Otoño
    23: { pal: ["#04142e", "#155e9e", "#38c6d9"], mode: 1, period: 22 }, // Inmersión
    24: { pal: ["#06261a", "#1f8f5f", "#a6e36b"], mode: 5, period: 14 }, // Jungla
    25: { pal: ["#0f3b2a", "#4fc38a", "#e8ffb0"], mode: 1, period: 16 }, // Mojito
    26: { pal: ["#1a0b3d", "#9b30ff", "#ff3fb4"], mode: 6, period: 6 }, // Club
    27: { pal: ["#0d3b1e", "#c8232c", "#ffd36b"], mode: 6, period: 12 }, // Navidad
    28: { pal: ["#1c0a2e", "#ff7a1a", "#ffc04d"], mode: 4, period: 1 }, // Halloween
  };

  const reduce = matchMedia("(prefers-reduced-motion: reduce)");
  const lin = (v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  const srgb = (v) => (v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055);
  const hexLin = (h) => [1, 3, 5].map((i) => lin(parseInt(h.slice(i, i + 2), 16) / 255));
  const linHex = (c) => `#${c.map((v) => Math.round(Math.min(1, Math.max(0, srgb(v))) * 255).toString(16).padStart(2, "0")).join("")}`;
  const lerp = (a, b, k) => a.map((v, i) => v + (b[i] - v) * k);
  const luma = ([r, g, b]) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

  /* Un color suelto (blancos y colores elegidos) se convierte en paleta tonal de su mismo tono. */
  function tonal(rgb) {
    const body = rgb.map((v) => lin(v / 255));
    return [body.map((v) => v * 0.16), body, lerp(body, [1, 1, 1], 0.55)];
  }

  function lookOf({ id, rgb, animated }) {
    const look = id ? LOOKS[id] : null;
    if (look) return { pal: look.pal.map(hexLin), mode: animated === false ? 0 : look.mode, period: look.period ?? 1 };
    return { pal: tonal(rgb ?? [245, 181, 68]), mode: 0, period: 1 };
  }

  const WGSL = /* wgsl */ `
struct U { res: vec2f, time: f32, power: f32, c0: vec4f, c1: vec4f, c2: vec4f,
           ign: vec2f, igt: f32, blend: f32, mode: f32, prev: f32, per: f32, pper: f32 };
@group(0) @binding(0) var<uniform> u: U;
const TAU = 6.2831853;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}
fn hash(p: vec2f) -> f32 { return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453); }
fn noise(p: vec2f) -> f32 {
  let i = floor(p); let f = fract(p); let s = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2f(1, 0)), s.x), mix(hash(i + vec2f(0, 1)), hash(i + vec2f(1, 1)), s.x), s.y);
}
fn fbm(p0: vec2f) -> f32 { var p = p0; var a = 0.5; var v = 0.0; for (var k = 0; k < 3; k++) { v += a * noise(p); p *= 2.03; a *= 0.5; } return v; }
// OKLab: mezclar aquí evita que el paso entre dos tonos se ensucie de gris.
fn toLab(c: vec3f) -> vec3f {
  let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
  let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
  let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
  let q = pow(max(vec3f(l, m, s), vec3f(0.0)), vec3f(1.0 / 3.0));
  return vec3f(0.2104542553 * q.x + 0.7936177850 * q.y - 0.0040720468 * q.z,
               1.9779984951 * q.x - 2.4285922050 * q.y + 0.4505937099 * q.z,
               0.0259040371 * q.x + 0.7827717662 * q.y - 0.8086757660 * q.z);
}
fn toLin(o: vec3f) -> vec3f {
  let l = o.x + 0.3963377774 * o.y + 0.2158037573 * o.z;
  let m = o.x - 0.1055613458 * o.y - 0.0638541728 * o.z;
  let s = o.x - 0.0894841775 * o.y - 1.2914855480 * o.z;
  let q = vec3f(l * l * l, m * m * m, s * s * s);
  return vec3f(4.0767416621 * q.x - 3.3077115913 * q.y + 0.2309699292 * q.z,
              -1.2684380046 * q.x + 2.6097574011 * q.y - 0.3413193965 * q.z,
              -0.0041960863 * q.x - 0.7034186147 * q.y + 1.7076147010 * q.z);
}
fn pal(t: f32) -> vec3f {
  let a = toLab(u.c0.rgb); let b = toLab(u.c1.rgb); let c = toLab(u.c2.rgb);
  let x = clamp(t, 0.0, 1.0);
  return toLin(select(mix(b, c, x * 2.0 - 1.0), mix(a, b, x * 2.0), x < 0.5));
}
// Cada bucle devuelve (posición en la paleta, intensidad). T recorre 0..TAU en un periodo.
fn sceneAt(id: i32, uv: vec2f, asp: vec2f, per: f32) -> vec2f {
  let T = TAU * fract(u.time / per);
  let ring = vec2f(cos(T), sin(T)); // ruido muestreado sobre un círculo = bucle sin corte
  let d = length((uv - vec2f(0.5, 0.36)) * asp);
  let glow = exp(-d * d * 2.6);
  let p = uv * asp;
  switch id {
    case 1: { // oleaje: cáusticas finas como en el fondo de una piscina
      let w = fbm(p * 2.4 + ring * 0.9);
      let a = abs(sin(p.x * 6.0 + w * 5.0 + T));
      let b = abs(sin(p.y * 5.0 - w * 4.0 + T * 2.0));
      return vec2f(glow * 0.7 + w * 0.25 + pow(1.0 - min(a, b), 6.0) * 0.45, 1.0);
    }
    case 2: { // ocaso: el sol baja y vuelve, el horizonte respira
      let sun = exp(-pow(length((uv - vec2f(0.5, 0.42 + 0.05 * sin(T))) * asp), 2.0) * 9.0);
      let sky = 1.0 - uv.y + 0.06 * sin(p.x * 3.0 + T) + fbm(p * 1.5 + ring * 0.4) * 0.15;
      return vec2f(sky * 0.65 + sun * 0.6, 1.0);
    }
    case 3: { // latido: dos pulsos y una pausa
      let b = fract(u.time / per);
      let beat = exp(-pow((b - 0.12) * 18.0, 2.0)) + 0.6 * exp(-pow((b - 0.3) * 18.0, 2.0));
      return vec2f(glow * (0.75 + 0.3 * beat) + fbm(p * 1.8 + ring * 0.2) * 0.12, 0.85 + 0.2 * beat);
    }
    case 4: { // brasa: y crece hacia abajo, así que el fuego nace abajo y sus lenguas suben
      let rise = fbm(vec2f(p.x * 3.0, p.y * 2.2 + u.time * 0.55));
      let tongues = smoothstep(0.15, 1.0, uv.y + (rise - 0.5) * 0.7);
      return vec2f(tongues * 0.95 + glow * 0.12, 0.9 + 0.1 * noise(vec2f(u.time * 7.0, 0.0)));
    }
    case 5: { // follaje: luz moteada entre hojas
      let dapple = smoothstep(0.42, 0.68, fbm(p * 4.5 + ring * 0.7));
      return vec2f(glow * 0.55 + dapple * 0.5, 1.0);
    }
    case 6: { // fiesta: la paleta recorre la pantalla
      let f = fract(p.x * 0.35 + p.y * 0.25 + u.time / per + fbm(p * 2.0 + ring) * 0.3);
      return vec2f(abs(f * 2.0 - 1.0) * 0.9 + glow * 0.15, 1.0);
    }
    case 7: { // amanecer: la luz sube desde abajo y se asienta
      let r = 0.5 - 0.5 * cos(T);
      let horizon = smoothstep(-0.1, 0.8, uv.y + r * 0.35 - 0.2);
      return vec2f(horizon * (0.5 + 0.5 * r) + glow * 0.3, 0.7 + 0.3 * r);
    }
    default: { // quieta: escenas fijas, solo la luz y su degradado
      return vec2f(glow * 0.85 + (1.0 - uv.y) * 0.2, 1.0);
    }
  }
}
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let uv = pos.xy / u.res;
  let asp = vec2f(u.res.x / u.res.y, 1.0);
  var s = sceneAt(i32(u.mode), uv, asp, u.per);
  if (u.blend < 0.999) { s = mix(sceneAt(i32(u.prev), uv, asp, u.pper), s, u.blend); }
  let d = length((uv - vec2f(0.5, 0.36)) * asp);
  let vign = smoothstep(1.25, 0.25, d);
  var c = pal(s.x) * (0.06 + 0.94 * u.power) * s.y * (0.25 + 0.75 * vign);
  let r = length((uv - u.ign) * asp);
  let front = u.igt * 1.6;
  c += pal(0.9) * smoothstep(front, front - 0.25, r) * (1.0 - u.igt) * 0.6;
  c = mix(vec3f(0.004), c, clamp(u.power * 4.0, 0.0, 1.0));
  var o = select(1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3f(0.0031308));
  o += (hash(pos.xy + fract(u.time)) - 0.5) / 255.0; // tramado contra bandas
  return vec4f(o, 1.0);
}`;

  const canvas = document.getElementById("lightCanvas");
  const fallback = document.getElementById("lightFallback");
  let gpu = null;
  let look = lookOf({});
  let prev = look;
  const now = { pal: look.pal.map((c) => [...c]), power: 0 };
  const want = { pal: look.pal, power: 0 };
  let blend = 1, igniteAt = -10, origin = [0.5, 0.36], active = true;

  function resize() {
    // shortcut: tercio de resolución fijo; subir a 0.5 si en pantallas grandes se nota borroso.
    canvas.width = Math.max(1, Math.round(innerWidth * 0.34));
    canvas.height = Math.max(1, Math.round(innerHeight * 0.34));
  }

  async function init() {
    const adapter = await navigator.gpu?.requestAdapter();
    if (!adapter) return null;
    const device = await adapter.requestDevice();
    const ctx = canvas.getContext("webgpu");
    const format = navigator.gpu.getPreferredCanvasFormat();
    ctx.configure({ device, format, alphaMode: "opaque" });
    const module = device.createShaderModule({ code: WGSL });
    const pipeline = device.createRenderPipeline({
      layout: "auto",
      vertex: { module, entryPoint: "vs" },
      fragment: { module, entryPoint: "fs", targets: [{ format }] },
    });
    const ubuf = device.createBuffer({ size: 96, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    const bind = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: ubuf } }] });
    // Si la GPU se pierde (driver, suspensión), la ventana sigue con el degradado CSS.
    device.lost.then(() => {
      gpu = null;
      canvas.hidden = true;
      fallback.hidden = false;
      wake();
    });
    return { device, ctx, pipeline, ubuf, bind, data: new Float32Array(24) };
  }

  let raf = 0, last = 0;
  function moving() {
    const settling = Math.abs(now.power - want.power) > 0.002 || blend < 1 ||
      now.pal.some((c, i) => c.some((v, j) => Math.abs(v - want.pal[i][j]) > 0.001));
    return settling || (look.mode && want.power > 0 && !reduce.matches) || performance.now() / 1000 - igniteAt < 1.1;
  }
  function wake() {
    if (!raf && active && !document.hidden) raf = requestAnimationFrame(frame);
  }
  function frame(ms) {
    raf = 0;
    if (!active) return;
    if (ms - last >= 33) {
      const dt = Math.min(100, ms - last);
      last = ms;
      const k = 1 - Math.exp(-dt / 220);
      now.power += (want.power - now.power) * k;
      now.pal = now.pal.map((c, i) => lerp(c, want.pal[i], k));
      blend = Math.min(1, blend + dt / 700);
      draw(ms / 1000);
    }
    if (moving()) raf = requestAnimationFrame(frame);
  }
  function draw(t) {
    if (!gpu) {
      fallback.style.setProperty("--c0", linHex(now.pal[0]));
      fallback.style.setProperty("--c1", linHex(now.pal[1]));
      fallback.style.setProperty("--p", (0.06 + now.power * 0.94).toFixed(3));
      return;
    }
    const [c0, c1, c2] = now.pal;
    gpu.data.set([canvas.width, canvas.height, reduce.matches ? 0 : t, now.power, ...c0, 0, ...c1, 0, ...c2, 0,
      ...origin, Math.min(1, t - igniteAt), blend, look.mode, prev.mode, look.period, prev.period]);
    gpu.device.queue.writeBuffer(gpu.ubuf, 0, gpu.data);
    const enc = gpu.device.createCommandEncoder();
    const pass = enc.beginRenderPass({
      colorAttachments: [{ view: gpu.ctx.getCurrentTexture().createView(), loadOp: "clear", storeOp: "store", clearValue: [0, 0, 0, 1] }],
    });
    pass.setPipeline(gpu.pipeline);
    pass.setBindGroup(0, gpu.bind);
    pass.draw(3);
    pass.end();
    gpu.device.queue.submit([enc.finish()]);
  }

  document.addEventListener("visibilitychange", wake);
  addEventListener("resize", () => {
    resize();
    wake();
  });
  resize();

  window.Light = {
    /* Pone la luz. Devuelve la luminancia que verá el texto encima (0..1). */
    set(target, power) {
      const next = lookOf(target);
      if (next.mode !== look.mode || next.period !== look.period) {
        prev = look;
        blend = 0;
      }
      look = next;
      want.pal = next.pal;
      want.power = Math.max(0, Math.min(1, power));
      wake();
      return luma(next.pal[1]) * (0.06 + 0.94 * want.power);
    },
    /* Frente de luz desde un punto de la ventana (coordenadas 0..1). */
    ignite(x = 0.5, y = 0.36) {
      origin = [x, y];
      igniteAt = performance.now() / 1000;
      wake();
      return Boolean(gpu);
    },
    /* Fuera de Inicio la luz no se ve: deja de dibujar. */
    active(on) {
      active = on;
      if (on) wake();
    },
    /* Paleta en hex para los puntos de escena del CSS. */
    palette(target) {
      return lookOf(target).pal.map(linHex);
    },
    get gpu() {
      return Boolean(gpu);
    },
    /* Para capturas en ventanas ocultas, donde requestAnimationFrame no corre: asienta y dibuja un fotograma. */
    renderNow(t = performance.now() / 1000) {
      now.power = want.power;
      now.pal = want.pal.map((c) => [...c]);
      blend = 1;
      igniteAt = -10;
      draw(t);
    },
  };

  init()
    .catch(() => null)
    .then((ready) => {
      gpu = ready;
      canvas.hidden = !gpu;
      fallback.hidden = Boolean(gpu);
      wake();
    });
})();
