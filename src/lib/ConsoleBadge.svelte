<script>
  // Badge de console posé sur la jaquette : silhouette de la console + son nom, aux couleurs de la marque.
  let { format = "", platform = "", small = false } = $props();

  // Extension (format) → console : plus précis que la plateforme (GBA ≠ Game Boy)
  const BY_EXT = {
    nsp: "switch", nsz: "switch", xci: "switch", xcz: "switch",
    "3ds": "3ds", cia: "3ds", cci: "3ds", cxi: "3ds",
    nds: "ds", dsi: "ds",
    gba: "gba", gbc: "gbc", gb: "gb",
    wbfs: "wii", rvz: "wii", wad: "wii", gcz: "wii", wia: "wii",
    wua: "wiiu",
    cso: "psp",
    pkg: "ps", pbp: "psp", chd: "retro",
  };
  // Nom de console (sites de ROM : « Nintendo DS », « Game Boy Advance », « PlayStation 2 (PS2) »…)
  function byPlatform(p) {
    const x = p.toLowerCase();
    if (/3ds/.test(x)) return "3ds";
    if (/switch/.test(x)) return "switch";
    if (/\bds\b|nds/.test(x)) return "ds";
    if (/advance|gba/.test(x)) return "gba";
    if (/color|gbc/.test(x)) return "gbc";
    if (/game ?boy|\bgb\b/.test(x)) return "gb";
    if (/wii ?u/.test(x)) return "wiiu";
    if (/wii/.test(x)) return "wii";
    if (/gamecube|\bgc\b|ngc/.test(x)) return "gc";
    if (/psp|portable/.test(x)) return "psp";
    if (/ps ?2|playstation 2/.test(x)) return "ps2";
    if (/ps ?3|playstation 3/.test(x)) return "ps3";
    if (/ps ?4|playstation 4/.test(x)) return "ps4";
    if (/ps ?1|psx|playstation/.test(x)) return "ps";
    if (/n64|nintendo 64/.test(x)) return "n64";
    if (/snes|super nintendo|super famicom/.test(x)) return "snes";
    if (/\bnes\b|famicom/.test(x)) return "nes";
    if (/mega ?drive|genesis/.test(x)) return "md";
    return null;
  }
  const C = {
    switch: { label: "Switch", color: "#e60012", shape: "switch" },
    "3ds": { label: "3DS", color: "#d6001c", shape: "clam" },
    ds: { label: "DS", color: "#c8ccd4", shape: "clam" },
    gba: { label: "GBA", color: "#7b5cff", shape: "wide" },
    gbc: { label: "GBC", color: "#f2b705", shape: "tall" },
    gb: { label: "Game Boy", color: "#9bbc0f", shape: "tall" },
    wii: { label: "Wii", color: "#5bc0f8", shape: "remote" },
    wiiu: { label: "Wii U", color: "#00a7e1", shape: "pad" },
    psp: { label: "PSP", color: "#4c8dff", shape: "psp" },
    ps: { label: "PS1", color: "#2e6cff", shape: "disc" },
    ps2: { label: "PS2", color: "#2e6cff", shape: "disc" },
    ps3: { label: "PS3", color: "#2e6cff", shape: "disc" },
    ps4: { label: "PS4", color: "#2e6cff", shape: "disc" },
    gc: { label: "GameCube", color: "#7c3aed", shape: "disc" },
    n64: { label: "N64", color: "#16a34a", shape: "pad" },
    snes: { label: "SNES", color: "#8b5cf6", shape: "pad" },
    nes: { label: "NES", color: "#dc2626", shape: "pad" },
    md: { label: "Mega Drive", color: "#111827", shape: "pad" },
    retro: { label: "Rétro", color: "#f59e0b", shape: "disc" },
  };
  const id = $derived(BY_EXT[(format ?? "").toLowerCase()] ?? byPlatform(platform) ?? null);
  const c = $derived(id ? C[id] : { label: platform || format.toUpperCase(), color: "#9ca3af", shape: "disc" });
</script>

<span class="cb" class:small style="--c:{c.color}" title={platform || c.label}>
  <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true">
    {#if c.shape === "clam"}
      <!-- Console à clapet : deux écrans -->
      <rect x="5" y="2" width="14" height="9.5" rx="2" fill="none" stroke="currentColor" stroke-width="1.8" />
      <rect x="7.5" y="4.5" width="9" height="4.5" rx="0.6" fill="var(--c)" />
      <rect x="5" y="12.5" width="14" height="9.5" rx="2" fill="none" stroke="currentColor" stroke-width="1.8" />
      <rect x="8.5" y="15" width="7" height="4.5" rx="0.6" fill="currentColor" opacity="0.55" />
    {:else if c.shape === "switch"}
      <!-- Joy-Con rouge et bleu autour de l'écran -->
      <path d="M7 4H5.5A3.5 3.5 0 0 0 2 7.5v9A3.5 3.5 0 0 0 5.5 20H7Z" fill="#00c3e3" />
      <path d="M17 4h1.5A3.5 3.5 0 0 1 22 7.5v9a3.5 3.5 0 0 1-3.5 3.5H17Z" fill="#ff4554" />
      <rect x="7.6" y="4" width="8.8" height="16" rx="0.8" fill="none" stroke="currentColor" stroke-width="1.6" />
    {:else if c.shape === "tall"}
      <rect x="6" y="2" width="12" height="20" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="1.8" />
      <rect x="8.3" y="4.5" width="7.4" height="6.5" rx="0.6" fill="var(--c)" />
      <circle cx="14.5" cy="15.5" r="1.3" fill="currentColor" />
      <path d="M8.5 15.5h3M10 14v3" stroke="currentColor" stroke-width="1.4" />
    {:else if c.shape === "wide"}
      <rect x="1.5" y="6" width="21" height="12" rx="5" fill="none" stroke="currentColor" stroke-width="1.8" />
      <rect x="8" y="8.3" width="8" height="7.4" rx="0.6" fill="var(--c)" />
      <path d="M3.8 12h2.4M5 10.8v2.4" stroke="currentColor" stroke-width="1.3" />
      <circle cx="19" cy="11.2" r="1" fill="currentColor" />
      <circle cx="18" cy="13.2" r="1" fill="currentColor" />
    {:else if c.shape === "psp"}
      <!-- PSP : corps arrondi, grand écran, croix à gauche, 4 boutons à droite -->
      <path d="M6.5 6.5h11a5.5 5.5 0 0 1 0 11h-11a5.5 5.5 0 0 1 0-11Z" fill="currentColor" fill-opacity="0.14" stroke="currentColor" stroke-width="1.5" />
      <rect x="7.6" y="8.4" width="8.8" height="7.2" rx="0.8" fill="var(--c)" />
      <path d="M2.9 12h2.8M4.3 10.6v2.8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <circle cx="19.7" cy="10.8" r="0.65" fill="currentColor" />
      <circle cx="19.7" cy="13.2" r="0.65" fill="currentColor" />
      <circle cx="18.5" cy="12" r="0.65" fill="currentColor" />
      <circle cx="20.9" cy="12" r="0.65" fill="currentColor" />
    {:else if c.shape === "remote"}
      <rect x="8" y="2" width="8" height="20" rx="3" fill="none" stroke="currentColor" stroke-width="1.8" />
      <path d="M10.5 6.5h3M12 5v3" stroke="var(--c)" stroke-width="1.6" />
      <circle cx="12" cy="11.5" r="1.4" fill="var(--c)" />
    {:else if c.shape === "pad"}
      <rect x="1.5" y="5" width="21" height="14" rx="4" fill="none" stroke="currentColor" stroke-width="1.8" />
      <rect x="7" y="7.5" width="10" height="9" rx="0.6" fill="var(--c)" />
    {:else}
      <!-- Disque -->
      <circle cx="12" cy="12" r="9.5" fill="none" stroke="currentColor" stroke-width="1.8" />
      <circle cx="12" cy="12" r="5.5" fill="none" stroke="var(--c)" stroke-width="2.2" />
      <circle cx="12" cy="12" r="1.6" fill="currentColor" />
    {/if}
  </svg>
  <b>{c.label}</b>
</span>

<style>
  .cb {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px 0 6px;
    border-radius: 999px;
    background: rgb(12 14 20 / 0.72);
    backdrop-filter: blur(8px);
    border: 1px solid color-mix(in srgb, var(--c) 55%, transparent);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.35);
    color: white;
    white-space: nowrap;
  }
  .cb svg { flex: none; }
  .cb.small { height: 20px; padding: 0 7px 0 4px; gap: 4px; box-shadow: none; background: #1b1f2a; }
  .cb.small b { font-size: 10.5px; }
  .cb b { font-size: 11px; font-weight: 800; letter-spacing: 0.02em; }
</style>
