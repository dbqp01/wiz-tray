/* PROTOTIPO, desechable: tres variantes de la vista principal para comparar en vivo.
   Pregunta: ¿cómo debería verse y moverse el cambio de escena?
   Solo se carga con ?variant= en la URL (modo navegador); la app de Tauri nunca la pasa.
   A = Luz que viaja · B = Recuadros vivos · C = Panel de cristal */
(() => {
  const VARIANTS = [
    ["A", "Luz que viaja"],
    ["B", "Recuadros vivos"],
    ["C", "Panel de cristal"],
  ];
  // Bucle de cada escena, copiado de light.js para animar los recuadros sin tocar el shader.
  const MODES = { 1: 1, 23: 1, 18: 1, 25: 1, 3: 2, 10: 2, 21: 2, 2: 3, 5: 4, 6: 4, 28: 4, 7: 5, 19: 5, 20: 5, 22: 5, 24: 5, 4: 6, 8: 6, 17: 6, 26: 6, 27: 6, 9: 7 };

  const css = document.createElement("link");
  css.rel = "stylesheet";
  css.href = "prototype.css";
  document.head.append(css);

  const params = new URLSearchParams(location.search);
  let index = Math.max(0, VARIANTS.findIndex(([key]) => key === params.get("variant")));

  const bar = document.createElement("div");
  bar.className = "proto-bar";
  const button = (text, name) => {
    const b = document.createElement("button");
    b.textContent = text;
    b.setAttribute("aria-label", name);
    return b;
  };
  const prevBtn = button("‹", "Variante anterior");
  const label = document.createElement("span");
  const nextBtn = button("›", "Variante siguiente");
  bar.append(prevBtn, label, nextBtn);
  document.body.append(bar);

  function apply() {
    const [key, name] = VARIANTS[index];
    document.body.dataset.variant = key;
    label.textContent = `${key} · ${name}`;
    params.set("variant", key);
    history.replaceState(null, "", `?${params}`);
    tagTiles();
  }
  const step = (delta) => {
    index = (index + delta + VARIANTS.length) % VARIANTS.length;
    apply();
  };
  prevBtn.addEventListener("click", () => step(-1));
  nextBtn.addEventListener("click", () => step(1));
  document.addEventListener("keydown", (event) => {
    const target = event.target;
    if (target.closest?.("input, textarea, select, [contenteditable], [role='slider']")) return;
    if (event.key === "ArrowLeft") step(-1);
    if (event.key === "ArrowRight") step(1);
  });

  /* B: cada recuadro anima su bucle con CSS; las pestañas reconstruyen la lista, así que se vuelve a marcar. */
  function tagTiles() {
    for (const tile of document.querySelectorAll("#sceneGrid .scene")) {
      tile.dataset.mode = MODES[tile.dataset.id] ?? 0;
    }
  }
  new MutationObserver(tagTiles).observe(document.getElementById("sceneGrid"), { childList: true });

  /* A: la luz de la escena nueva se abre desde el recuadro pulsado. */
  document.getElementById("sceneGrid").addEventListener("click", (event) => {
    if (document.body.dataset.variant !== "A") return;
    const tile = event.target.closest(".scene");
    if (!tile) return;
    const box = tile.getBoundingClientRect();
    window.Light.ignite((box.left + box.width / 2) / innerWidth, (box.top + box.height / 2) / innerHeight);
    tile.classList.remove("proto-press");
    void tile.offsetWidth;
    tile.classList.add("proto-press");
  });

  apply();
})();
