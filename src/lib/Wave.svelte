<script>
  // Courbe du son dessinée dans une timeline : un pic par colonne de pixels, pour la portion [from, to] (secondes)
  /** peaks : valeurs 0..255 ; perSec : points par seconde ; from / to : portion visible */
  let { peaks = null, perSec = 50, from = 0, to = 1 } = $props();

  let canvas = $state();
  let w = $state(0);
  let h = $state(0);

  $effect(() => {
    if (!canvas || !w || !h) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    if (!peaks?.length || to <= from) return;
    const color = getComputedStyle(canvas).color;
    ctx.fillStyle = color;
    const span = to - from;
    const mid = h / 2;
    for (let x = 0; x < w; x++) {
      const a = Math.floor((from + (x / w) * span) * perSec);
      const b = Math.max(a + 1, Math.floor((from + ((x + 1) / w) * span) * perSec));
      let m = 0;
      for (let i = Math.max(0, a); i < Math.min(peaks.length, b); i++) if (peaks[i] > m) m = peaks[i];
      const bar = Math.max(1, (m / 255) * (h - 4));
      ctx.fillRect(x, mid - bar / 2, 1, bar);
    }
  });
</script>

<div class="wave" bind:clientWidth={w} bind:clientHeight={h}>
  <canvas bind:this={canvas} style="width:{w}px;height:{h}px"></canvas>
</div>

<style>
  .wave { position: absolute; inset: 0; pointer-events: none; color: var(--wave, rgb(76 194 255 / 0.55)); }
  canvas { display: block; }
</style>
