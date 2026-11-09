<script>
  // Rangée défilante : flèches sur les côtés, glisser à la souris (avec élan), pas de barre de défilement
  import Icon from "./Icon.svelte";

  let { children, arrowTop = 92 } = $props();

  let el = $state();
  let canL = $state(false);
  let canR = $state(false);
  let dragging = $state(false);

  function update() {
    if (!el) return;
    canL = el.scrollLeft > 4;
    canR = el.scrollLeft + el.clientWidth < el.scrollWidth - 4;
  }

  $effect(() => {
    if (!el) return;
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    const mo = new MutationObserver(update);
    mo.observe(el, { childList: true, subtree: true });
    return () => (ro.disconnect(), mo.disconnect());
  });

  function page(dir) {
    el.scrollBy({ left: dir * el.clientWidth * 0.85, behavior: "smooth" });
  }

  // Glisser pour faire défiler ; un vrai glissement annule le clic sur la carte
  let start = 0, from = 0, moved = false, lastX = 0, lastT = 0, vel = 0, raf = 0;
  function down(e) {
    if (e.button !== 0) return;
    cancelAnimationFrame(raf);
    start = lastX = e.clientX;
    lastT = performance.now();
    from = el.scrollLeft;
    moved = false;
    vel = 0;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
  }
  function move(e) {
    const dx = e.clientX - start;
    if (!moved && Math.abs(dx) < 6) return;
    if (!moved) ((moved = true), (dragging = true));
    el.scrollLeft = from - dx;
    const now = performance.now();
    const dt = Math.max(1, now - lastT);
    vel = 0.8 * ((lastX - e.clientX) / dt) + 0.2 * vel;
    lastX = e.clientX;
    lastT = now;
  }
  function up() {
    window.removeEventListener("pointermove", move);
    if (!moved) return;
    // élan
    let v = vel * 16;
    const step = () => {
      if (Math.abs(v) < 0.5) return (dragging = false);
      el.scrollLeft += v;
      v *= 0.92;
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
  }
  function click(e) {
    if (moved) {
      e.stopPropagation();
      e.preventDefault();
      moved = false;
    }
  }
</script>

<div class="car" style="--arrow-top: {arrowTop}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="track" class:dragging bind:this={el} onscroll={update} onpointerdown={down} onclickcapture={click} ondragstart={(e) => e.preventDefault()}>
    {@render children()}
  </div>
  {#if canL}
    <button class="arrow l" onclick={() => page(-1)} aria-label="Précédent"><Icon name="chevron" size={18} /></button>
  {/if}
  {#if canR}
    <button class="arrow r" onclick={() => page(1)} aria-label="Suivant"><Icon name="chevron" size={18} /></button>
  {/if}
  <span class="fade l" class:on={canL}></span>
  <span class="fade r" class:on={canR}></span>
</div>

<style>
  .car { position: relative; }
  .track { display: flex; gap: 12px; overflow-x: auto; padding: 4px 2px 6px; scrollbar-width: none; cursor: grab; user-select: none; }
  .track::-webkit-scrollbar { display: none; }
  .track.dragging { cursor: grabbing; }
  .track.dragging :global(*) { pointer-events: none; }
  .track :global(img) { -webkit-user-drag: none; }
  .fade { position: absolute; top: 0; bottom: 0; width: 56px; pointer-events: none; opacity: 0; transition: opacity 0.2s; z-index: 1; }
  .fade.on { opacity: 1; }
  .fade.l { left: 0; background: linear-gradient(90deg, var(--fade-bg, var(--bg-2)), transparent); }
  .fade.r { right: 0; background: linear-gradient(-90deg, var(--fade-bg, var(--bg-2)), transparent); }
  .arrow { position: absolute; top: var(--arrow-top); z-index: 2; transform: translateY(-50%); display: grid; place-items: center; width: 40px; height: 40px; border-radius: 50%; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); box-shadow: 0 4px 14px rgb(0 0 0 / 0.18); cursor: pointer; transition: transform 0.15s, background 0.15s, color 0.15s; }
  .arrow:hover { background: var(--accent); border-color: var(--accent); color: white; transform: translateY(-50%) scale(1.08); }
  .arrow.l { left: 6px; }
  .arrow.l :global(svg) { transform: rotate(180deg); }
  .arrow.r { right: 6px; }
</style>
