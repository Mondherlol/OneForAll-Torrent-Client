<script>
  // Analyse d'un disque façon SpaceSniffer : chaque bloc a la taille de ce qu'il occupe.
  // Clic = entrer dans le dossier ; clic droit = ouvrir / afficher / mettre à la corbeille.
  import { fade, scale } from "svelte/transition";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "./Icon.svelte";
  import { diskAnalyze, diskAnalyzeCancel, openPath, reveal, trashPath } from "./api.js";
  import { KIND_LABEL, fmtSize } from "./model.js";
  import { confirmAsk, flash, openMenu } from "./ui.svelte.js";

  /** mount : « C:\ » ; used : place occupée ; torrents / downloads : pour nommer ce qu'on reconnaît */
  let { mount, used = 0, torrents = [], downloads = [], onclose } = $props();

  const CATS = [
    { id: "video", label: "Vidéos", hue: 214 },
    { id: "audio", label: "Musique", hue: 330 },
    { id: "image", label: "Images", hue: 152 },
    { id: "archive", label: "Archives", hue: 36 },
    { id: "app", label: "Programmes", hue: 266 },
    { id: "game", label: "Données de jeu", hue: 4 },
    { id: "doc", label: "Documents", hue: 186 },
    { id: "other", label: "Divers", hue: 220, gray: true },
  ];
  const domCat = (n) => {
    let best = 7;
    for (let i = 0; i < 8; i++) if (n.cats[i] > n.cats[best]) best = i;
    return CATS[best];
  };
  const color = (n, a = 1) => {
    const c = domCat(n);
    return c.gray ? `hsl(220 10% 62% / ${a})` : `hsl(${c.hue} 68% 56% / ${a})`;
  };

  // ---------- analyse ----------
  let tree = $state.raw(null);
  let stack = $state.raw([]);
  let progress = $state({ files: 0, bytes: 0 });
  let err = $state("");
  let version = $state(0);
  const t0 = Date.now();
  let elapsed = $state(0);

  async function run() {
    tree = null;
    err = "";
    progress = { files: 0, bytes: 0 };
    const un = await listen("disk-scan", (e) => (progress = e.payload));
    const tick = setInterval(() => (elapsed = Math.round((Date.now() - t0) / 1000)), 500);
    try {
      const t = await diskAnalyze(mount);
      tree = t;
      stack = [t];
    } catch (e) {
      err = String(e);
    } finally {
      un();
      clearInterval(tick);
    }
  }
  run();

  function close() {
    if (!tree) diskAnalyzeCancel().catch(() => {});
    onclose();
  }

  // ---------- noms reconnus ----------
  const base = mount.replace(/\\$/, "");
  const node = $derived(stack[stack.length - 1]);
  const pathOf = (child, at = stack) => [base, ...at.slice(1).map((n) => n.name), ...(child ? [child.name] : [])].join("\\");
  const known = $derived.by(() => {
    const m = new Map();
    for (const t of torrents) if (t.content_path) m.set(t.content_path.toLowerCase().replace(/\\$/, ""), { title: t.title, kind: t.kind });
    for (const d of downloads) if (d.path) m.set(d.path.toLowerCase(), { title: d.title, kind: d.kind });
    return m;
  });
  /** { title, tag } : ce qu'on sait dire de ce dossier ou fichier */
  function describe(n, path) {
    const low = path.toLowerCase();
    const k = known.get(low);
    if (k) return { title: k.title, tag: KIND_LABEL[k.kind] ?? "" };
    const parent = low.split("\\").slice(-2, -1)[0] ?? "";
    const name = n.name.toLowerCase();
    if (/steamapps\\common\\[^\\]+$/.test(low)) return { title: n.name, tag: "Jeu Steam" };
    if (/\\(epic games|riot games|gog games|ubisoft game launcher\\games|ea games)\\[^\\]+$/.test(low) || /^[a-z]:\\(riot games|epic games)$/.test(low)) return { title: n.name, tag: "Jeu" };
    if (/^program files/.test(parent)) return { title: n.name, tag: "Logiciel" };
    if (/^[a-z]:\\windows$/.test(low)) return { title: "Windows", tag: "Système" };
    if (name === "hiberfil.sys") return { title: "Mise en veille prolongée", tag: "Système" };
    if (name === "pagefile.sys" || name === "swapfile.sys") return { title: "Mémoire virtuelle", tag: "Système" };
    if (name === "$recycle.bin") return { title: "Corbeille", tag: "" };
    if (name === "appdata") return { title: "AppData", tag: "Données d'applis" };
    if (/^[a-z]:\\users$/.test(low)) return { title: "Utilisateurs", tag: "" };
    if (/^program files/.test(name)) return { title: n.name, tag: "Logiciels" };
    if (name === "steamlibrary" || name === "steamapps") return { title: n.name, tag: "Jeux Steam" };
    const c = domCat(n);
    const share = n.size ? n.cats[CATS.indexOf(c)] / n.size : 0;
    return { title: n.name, tag: !c.gray && share > 0.6 ? c.label : "" };
  }

  // ---------- disposition (treemap « squarified ») ----------
  function squarify(items, x, y, w, h) {
    const total = items.reduce((a, b) => a + b.size, 0);
    if (!total || w < 2 || h < 2) return [];
    const k = (w * h) / total;
    const rest = items.map((n) => ({ n, a: n.size * k }));
    const out = [];
    let row = [];
    let rx = x, ry = y, rw = w, rh = h;
    const worst = (r, side) => {
      const s = r.reduce((a, b) => a + b.a, 0);
      let mx = 0, mn = Infinity;
      for (const i of r) ((mx = Math.max(mx, i.a)), (mn = Math.min(mn, i.a)));
      return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
    };
    const place = () => {
      const s = row.reduce((a, b) => a + b.a, 0);
      if (rw >= rh) {
        const cw = s / rh;
        let cy = ry;
        for (const r of row) out.push({ n: r.n, x: rx, y: cy, w: cw, h: r.a / cw }), (cy += r.a / cw);
        rx += cw;
        rw -= cw;
      } else {
        const ch = s / rw;
        let cx = rx;
        for (const r of row) out.push({ n: r.n, x: cx, y: ry, w: r.a / ch, h: ch }), (cx += r.a / ch);
        ry += ch;
        rh -= ch;
      }
      row = [];
    };
    while (rest.length) {
      const side = Math.min(rw, rh);
      if (!row.length || worst([...row, rest[0]], side) <= worst(row, side)) row.push(rest.shift());
      else place();
    }
    if (row.length) place();
    return out;
  }
  // Ce qui est montré dans un dossier : ses gros éléments + « petits fichiers » s'ils pèsent
  const REST = (n) => ({ name: "Petits éléments", size: n.rest, cats: n.cats, files: 0, dir: false, children: [], rest: 0, isRest: true });
  const shown = (n) => [...n.children, ...(n.rest > n.size * 0.01 ? [REST(n)] : [])].sort((a, b) => b.size - a.size);

  let W = $state(0);
  let H = $state(0);
  const HEAD = 24;
  const blocks = $derived.by(() => {
    version;
    if (!node || !W || !H) return [];
    return squarify(shown(node), 0, 0, W, H).map((b) => {
      const path = pathOf(b.n);
      const big = b.n.dir && !b.n.isRest && b.w > 130 && b.h > 80 && b.n.children.length;
      return {
        ...b,
        path,
        info: b.n.isRest ? { title: "Petits éléments", tag: `${node.files.toLocaleString("fr-FR")} fichiers en tout` } : describe(b.n, path),
        kids: big ? squarify(shown(b.n), 3, HEAD, b.w - 6, b.h - HEAD - 3) : [],
      };
    });
  });

  // ---------- actions ----------
  function enter(n) {
    if (!n.dir || n.isRest || !n.children.length) return;
    stack = [...stack, n];
  }
  function enterKid(parent, kid) {
    if (!kid.dir || kid.isRest || !kid.children.length) return enter(parent);
    stack = [...stack, parent, kid];
  }
  const up = (i) => (stack = stack.slice(0, i + 1));

  async function trash(n, owner, path) {
    const ok = await confirmAsk(`Mettre « ${n.name} » à la corbeille ?`, `${fmtSize(n.size)} · ${path}`, "Mettre à la corbeille", true);
    if (!ok) return;
    try {
      await trashPath(path);
      owner.children = owner.children.filter((c) => c !== n);
      // Les dossiers au-dessus perdent sa taille
      const i = stack.indexOf(owner);
      const chain = i >= 0 ? stack.slice(0, i + 1) : [...stack, owner];
      for (const a of chain) {
        a.size -= n.size;
        a.files -= n.files;
        for (let i = 0; i < 8; i++) a.cats[i] = Math.max(0, a.cats[i] - n.cats[i]);
      }
      version++;
      flash(`${n.name} mis à la corbeille · ${fmtSize(n.size)} libérés`);
    } catch (e) {
      flash(String(e));
    }
  }
  function menu(e, n, owner, path) {
    e.preventDefault();
    e.stopPropagation();
    if (n.isRest) return;
    openMenu(e, [
      n.dir && n.children.length && { label: "Entrer", icon: "layers", run: () => (owner === node ? enter(n) : enterKid(owner, n)) },
      { label: "Ouvrir", icon: "open", run: () => openPath(path).catch((er) => flash(String(er))) },
      { label: "Afficher dans l'Explorateur", icon: "folder", run: () => reveal(path, null).catch((er) => flash(String(er))) },
      "-",
      { label: "Mettre à la corbeille…", icon: "trash", danger: true, run: () => trash(n, owner, path) },
    ]);
  }

  // Survol : chemin complet et détails
  let hover = $state(null);
  function key(e) {
    if (e.key === "Escape") close();
    else if (e.key === "Backspace" && stack.length > 1) up(stack.length - 2);
  }
  const pct = $derived(used ? Math.min(100, (progress.bytes / used) * 100) : 0);
</script>

<svelte:window onkeydown={key} />

<div class="backdrop" transition:fade={{ duration: 120 }} onclick={close} role="presentation"></div>
<div class="win" transition:scale={{ start: 0.98, duration: 150 }} role="dialog" aria-label="Analyse du disque">
  <header>
    <span class="ic"><Icon name="drive" size={18} /></span>
    <div class="crumbs">
      {#if tree}
        {#each stack as s, i}
          {#if i}<Icon name="chevron" size={13} />{/if}
          <button class:on={i === stack.length - 1} onclick={() => up(i)}>{i ? s.name : base}</button>
        {/each}
        <span class="tot">{fmtSize(node.size)} · {node.files.toLocaleString("fr-FR")} fichiers</span>
      {:else}
        <b>Analyse de {base}</b>
      {/if}
    </div>
    {#if tree}<button class="hb" title="Analyser de nouveau" onclick={run}><Icon name="refresh" size={15} /></button>{/if}
    <button class="hb" title="Fermer (Échap)" onclick={close}><Icon name="close" size={16} /></button>
  </header>

  {#if err}
    <div class="state"><Icon name="alert" size={30} /><p>{err}</p></div>
  {:else if !tree}
    <div class="state">
      <div class="scan-ring"><span></span></div>
      <h3>Analyse de {base}…</h3>
      <p>{progress.files.toLocaleString("fr-FR")} fichiers · {fmtSize(progress.bytes)}{used ? ` sur ${fmtSize(used)}` : ""}</p>
      {#if used}<div class="pbar"><span style="width:{pct}%"></span></div>{/if}
      <small>{elapsed} s — environ une minute pour un disque bien rempli</small>
    </div>
  {:else}
    <div class="map" bind:clientWidth={W} bind:clientHeight={H}>
      {#each blocks as b (b.path)}
        {@const c = domCat(b.n)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="blk"
          class:dir={b.n.dir && !b.n.isRest}
          class:rest={b.n.isRest}
          class:nested={b.kids.length}
          style="left:{b.x}px;top:{b.y}px;width:{b.w}px;height:{b.h}px;--c:{color(b.n)};--cs:{color(b.n, 0.16)}"
          role="button"
          tabindex="-1"
          onclick={() => enter(b.n)}
          oncontextmenu={(e) => menu(e, b.n, node, b.path)}
          onmouseenter={() => (hover = b)}
          onmouseleave={() => hover === b && (hover = null)}
        >
          {#if b.w > 46 && b.h > 20}
            <div class="lbl" class:small={b.h < 44 || b.w < 90}>
              {#if b.n.dir && !b.n.isRest && b.w > 90}<Icon name="folder" size={12} />{/if}
              <b>{b.info.title}</b>
              {#if b.w > 110}<span class="sz">{fmtSize(b.n.size)}</span>{/if}
            </div>
            {#if b.info.tag && !b.kids.length && b.h > 50 && b.w > 80}<span class="tag">{b.info.tag}</span>{/if}
            {#if b.info.tag && b.kids.length}<span class="tag top">{b.info.tag}</span>{/if}
          {/if}
          {#each b.kids as k (k.n.name + k.x)}
            {@const kp = b.path + "\\" + k.n.name}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="kid"
              class:rest={k.n.isRest}
              style="left:{k.x}px;top:{k.y}px;width:{k.w}px;height:{k.h}px;--c:{color(k.n)}"
              role="button"
              tabindex="-1"
              title="{k.n.name} · {fmtSize(k.n.size)}"
              onclick={(e) => (e.stopPropagation(), enterKid(b.n, k.n))}
              oncontextmenu={(e) => menu(e, k.n, b.n, kp)}
            >
              {#if k.w > 54 && k.h > 26}
                <span class="kl">{k.n.isRest ? "Petits éléments" : describe(k.n, kp).title}</span>
                {#if k.h > 40 && k.w > 60}<span class="ks">{fmtSize(k.n.size)}</span>{/if}
              {/if}
            </div>
          {/each}
        </div>
      {/each}
    </div>
    <footer>
      <div class="legend">
        {#each CATS as c, i}
          {#if node.cats[i] > node.size * 0.005}
            <span><i style="background:{c.gray ? 'hsl(220 10% 62%)' : `hsl(${c.hue} 68% 56%)`}"></i>{c.label} <b>{fmtSize(node.cats[i])}</b></span>
          {/if}
        {/each}
      </div>
      <span class="hint">
        {#if hover}<b>{hover.path}</b> · {fmtSize(hover.n.size)}{#if hover.n.files > 1} · {hover.n.files.toLocaleString("fr-FR")} fichiers{/if}{:else}Clic : entrer · clic droit : ouvrir, supprimer · Retour arrière : remonter{/if}
      </span>
    </footer>
  {/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 90; background: var(--scrim); backdrop-filter: blur(3px); }
  .win { position: fixed; z-index: 91; inset: 4vh 3vw; display: flex; flex-direction: column; border-radius: 18px; overflow: hidden; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); }
  header { display: flex; align-items: center; gap: 10px; padding: 12px 14px 12px 18px; border-bottom: 1px solid var(--line); }
  .ic { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 10px; background: var(--accent-soft); color: var(--accent); flex: none; }
  .crumbs { flex: 1; min-width: 0; display: flex; align-items: center; gap: 2px; overflow: hidden; color: var(--muted); }
  .crumbs b { font-size: 15px; color: var(--text); }
  .crumbs button { border: 0; background: none; font: inherit; font-size: 14px; font-weight: 600; color: var(--text-2); padding: 5px 8px; border-radius: 7px; cursor: pointer; white-space: nowrap; max-width: 260px; overflow: hidden; text-overflow: ellipsis; }
  .crumbs button:hover { background: var(--chip); color: var(--accent); }
  .crumbs button.on { color: var(--text); font-weight: 750; font-size: 15px; }
  .tot { margin-left: 10px; font-size: 12.5px; white-space: nowrap; }
  .hb { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); cursor: pointer; flex: none; }
  .hb:hover { color: var(--accent); border-color: var(--accent); }

  .state { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: var(--muted); text-align: center; padding: 20px; }
  .state h3 { margin: 10px 0 0; color: var(--text); font-size: 18px; }
  .state p { margin: 0; font-size: 14px; font-variant-numeric: tabular-nums; }
  .state small { font-size: 12px; }
  .pbar { width: min(420px, 70%); height: 8px; border-radius: 8px; background: var(--chip); overflow: hidden; margin: 6px 0; }
  .pbar span { display: block; height: 100%; background: var(--accent); border-radius: 8px; transition: width 0.25s; }
  .scan-ring { width: 64px; height: 64px; border-radius: 50%; border: 5px solid var(--chip); border-top-color: var(--accent); animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .map { position: relative; flex: 1; margin: 10px; min-height: 0; }
  .blk { position: absolute; box-sizing: border-box; border: 2px solid var(--bg-2); border-radius: 7px; overflow: hidden; cursor: pointer; background: linear-gradient(160deg, color-mix(in srgb, var(--c) 100%, white 12%), var(--c)); color: white; transition: filter 0.12s; }
  .blk.nested { background: var(--cs); border-color: var(--bg-2); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 55%, transparent); color: var(--text); }
  .blk.rest { background: repeating-linear-gradient(135deg, hsl(220 10% 60% / 0.35) 0 6px, hsl(220 10% 60% / 0.2) 6px 12px); color: var(--text-2); cursor: default; }
  .blk:hover { filter: brightness(1.08); }
  .blk.nested:hover { box-shadow: inset 0 0 0 2px var(--c); filter: none; }
  .lbl { position: relative; z-index: 1; display: flex; align-items: center; gap: 5px; height: 22px; padding: 3px 8px 0; font-size: 12.5px; white-space: nowrap; overflow: hidden; }
  .lbl b { overflow: hidden; text-overflow: ellipsis; font-weight: 700; }
  .lbl .sz { margin-left: auto; padding-left: 6px; font-weight: 600; opacity: 0.85; font-variant-numeric: tabular-nums; }
  .lbl.small { font-size: 11px; padding-top: 2px; }
  .blk:not(.nested) .lbl { text-shadow: 0 1px 2px rgb(0 0 0 / 0.35); }
  .tag { position: absolute; left: 8px; bottom: 6px; padding: 1px 7px; border-radius: 5px; background: rgb(0 0 0 / 0.28); font-size: 10.5px; font-weight: 700; }
  .tag.top { position: absolute; right: 6px; left: auto; top: 3px; bottom: auto; background: var(--c); color: white; z-index: 2; }
  .kid { position: absolute; box-sizing: border-box; border: 1.5px solid color-mix(in srgb, var(--bg-2) 70%, transparent); border-radius: 5px; overflow: hidden; background: linear-gradient(160deg, color-mix(in srgb, var(--c) 100%, white 10%), var(--c)); color: white; padding: 3px 6px; display: flex; flex-direction: column; gap: 1px; }
  .kid:hover { filter: brightness(1.12); }
  .kid.rest { background: repeating-linear-gradient(135deg, hsl(220 10% 60% / 0.45) 0 5px, hsl(220 10% 60% / 0.28) 5px 10px); color: var(--text-2); }
  .kl { font-size: 11px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; text-shadow: 0 1px 2px rgb(0 0 0 / 0.3); }
  .ks { font-size: 10.5px; opacity: 0.85; font-variant-numeric: tabular-nums; }

  footer { display: flex; align-items: center; gap: 16px; padding: 8px 16px 12px; font-size: 12px; color: var(--muted); }
  .legend { display: flex; flex-wrap: wrap; gap: 12px; }
  .legend span { display: inline-flex; align-items: center; gap: 5px; }
  .legend i { width: 10px; height: 10px; border-radius: 3px; }
  .legend b { color: var(--text-2); font-weight: 650; }
  .hint { margin-left: auto; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .hint b { color: var(--text-2); }
</style>
