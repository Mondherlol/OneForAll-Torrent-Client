<script>
  // Analyse d'un disque façon SpaceSniffer : chaque bloc a la taille de ce qu'il occupe.
  // Clic = entrer dans le dossier ; clic droit = ouvrir / afficher / mettre à la corbeille.
  import { fade, scale, slide } from "svelte/transition";
  import { tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "./Icon.svelte";
  import { diskAnalyze, diskAnalyzeCancel, openPath, reveal, trashPath } from "./api.js";
  import { KIND_LABEL, fmtSize } from "./model.js";
  import { confirmAsk, flash, openMenu } from "./ui.svelte.js";

  /** mount : « C:\ » ; used : place occupée ; torrents / downloads : pour nommer ce qu'on reconnaît */
  let { mount, used = 0, torrents = [], downloads = [], onclose } = $props();
  const base = mount.replace(/\\$/, "");

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

  // Pendant l'analyse : arbre provisoire rempli au fur et à mesure (dossiers terminés des 3 premiers niveaux)
  const blank = (name) => ({ name, size: 0, files: 0, cats: [0, 0, 0, 0, 0, 0, 0, 0], dir: true, children: [], rest: 0, live: true });
  let live = $state.raw(null);
  let scanning = $state(false);
  function addPart({ path, node: n }) {
    if (!live) return;
    let at = live;
    const chain = [live];
    for (const name of path.slice(0, -1)) {
      let c = at.children.find((x) => x.name === name);
      if (!c) at.children.push((c = blank(name)));
      chain.push((at = c));
    }
    const last = path[path.length - 1];
    const i = at.children.findIndex((x) => x.name === last);
    // Un dossier fini remplace sa version provisoire (ses sous-dossiers déjà reçus restent plus détaillés)
    const prev = i >= 0 ? at.children[i] : null;
    const done = { ...n, children: prev?.live ? mergeKids(n.children, prev.children) : n.children };
    if (i >= 0) at.children[i] = done;
    else at.children.push(done);
    // Les dossiers parents provisoires : somme de ce qui est déjà fini
    for (const a of chain.reverse()) {
      if (!a.live) continue;
      a.size = a.children.reduce((s, c) => s + c.size, 0);
      a.files = a.children.reduce((s, c) => s + c.files, 0);
      a.cats = a.cats.map((_, k) => a.children.reduce((s, c) => s + c.cats[k], 0));
      a.children.sort((x, y) => y.size - x.size);
    }
    live = { ...live };
  }
  const mergeKids = (fresh, old) => fresh.map((f) => old.find((o) => o.name === f.name && o.children?.length) ?? f);

  async function run() {
    tree = null;
    err = "";
    progress = { files: 0, bytes: 0 };
    live = blank(base);
    stack = [];
    scanning = true;
    const un = await listen("disk-scan", (e) => (progress = e.payload));
    const un2 = await listen("disk-part", (e) => addPart(e.payload));
    const tick = setInterval(() => (elapsed = Math.round((Date.now() - t0) / 1000)), 500);
    try {
      const t = await diskAnalyze(mount);
      tree = t;
      stack = [t];
    } catch (e) {
      err = String(e);
    } finally {
      un();
      un2();
      clearInterval(tick);
      scanning = false;
      live = null;
    }
  }
  run();

  function close() {
    if (!tree) diskAnalyzeCancel().catch(() => {});
    onclose();
  }

  // ---------- noms reconnus ----------
  const node = $derived(scanning && live ? live : stack[stack.length - 1]);
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
  const shown = (n) =>
    [...n.children, ...(n.rest > n.size * 0.01 ? [REST(n)] : [])].sort((a, b) => b.size - a.size);

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
        info: b.n.pending ? { title: "Analyse en cours…", tag: "" } : b.n.isRest ? { title: "Petits éléments", tag: `${node.files.toLocaleString("fr-FR")} fichiers en tout` } : describe(b.n, path),
        kids: big ? squarify(shown(b.n), 3, HEAD, b.w - 6, b.h - HEAD - 3) : [],
      };
    });
  });

  // ---------- actions ----------
  // Entrer / sortir d'un dossier : la carte zoome depuis (ou vers) le bloc du dossier
  let inner = $state();
  const EASE = "cubic-bezier(.2,.8,.2,1)";
  function zoomIn(r) {
    if (!inner || !r) return;
    inner.animate(
      [{ transform: `translate(${r.x}px, ${r.y}px) scale(${r.w / W}, ${r.h / H})`, opacity: 0.4 }, { transform: "none", opacity: 1 }],
      { duration: 340, easing: EASE },
    );
  }
  function zoomOut(r) {
    if (!inner || !r) return;
    const sx = W / r.w, sy = H / r.h;
    inner.animate(
      [{ transform: `translate(${-r.x * sx}px, ${-r.y * sy}px) scale(${sx}, ${sy})`, opacity: 0.4 }, { transform: "none", opacity: 1 }],
      { duration: 340, easing: EASE },
    );
  }
  const canEnter = (n) => !scanning && n.dir && !n.isRest && n.children.length;
  async function enter(n, r) {
    if (!canEnter(n)) return;
    stack = [...stack, n];
    await tick();
    zoomIn(r);
  }
  async function enterKid(parent, kid, r) {
    if (!canEnter(kid)) return enter(parent, r?.parent);
    stack = [...stack, parent, kid];
    await tick();
    zoomIn(r);
  }
  async function up(i) {
    if (i >= stack.length - 1) return;
    const from = stack[i + 1];
    stack = stack.slice(0, i + 1);
    await tick();
    const b = blocks.find((x) => x.n === from);
    zoomOut(b);
  }

  // ---------- vue liste : les plus lourds du disque ----------
  let view = $state("blocks");
  const SPECIFIC_TAGS = ["Jeu Steam", "Jeu", "Logiciel", "Système"];
  /** Les éléments les plus lourds « qui veulent dire quelque chose » (un jeu, une série, un gros fichier…),
   * sans compter deux fois un dossier et ce qu'il contient */
  const heavy = $derived.by(() => {
    version;
    if (!tree) return [];
    const all = [];
    const walk = (n, path, chain) => {
      for (const c of n.children) {
        const p = `${path}\\${c.name}`;
        const info = describe(c, p);
        const dirKids = c.children.filter((k) => k.dir).reduce((s, k) => s + k.size, 0);
        const filesShare = c.size ? 1 - dirKids / c.size : 0;
        const specific = !c.dir || known.has(p.toLowerCase()) || SPECIFIC_TAGS.includes(info.tag) || (chain.length >= 1 && filesShare >= 0.6);
        if (specific && c.size > 0) all.push({ n: c, path: p, info, owner: n, chain: [...chain, n] });
        if (c.dir) walk(c, p, [...chain, n]);
      }
    };
    walk(tree, base, []);
    all.sort((a, b) => b.n.size - a.n.size);
    const picked = [];
    for (const h of all) {
      const lp = h.path.toLowerCase();
      if (picked.some((x) => lp.startsWith(x.lp + "\\") || x.lp.startsWith(lp + "\\"))) continue;
      picked.push({ ...h, lp });
      if (picked.length >= 60) break;
    }
    return picked;
  });
  // Depuis la liste : on ouvre les blocs sur le dossier qui contient l'élément
  async function goTo(h) {
    view = "blocks";
    stack = h.chain;
    await tick();
  }

  async function trash(n, owner, path, ancestors = null) {
    const ok = await confirmAsk(`Mettre « ${n.name} » à la corbeille ?`, `${fmtSize(n.size)} · ${path}`, "Mettre à la corbeille", true);
    if (!ok) return;
    try {
      await trashPath(path);
      owner.children = owner.children.filter((c) => c !== n);
      // Les dossiers au-dessus perdent sa taille
      const i = stack.indexOf(owner);
      const chain = ancestors ?? (i >= 0 ? stack.slice(0, i + 1) : [...stack, owner]);
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
  function menu(e, n, owner, path, ancestors = null) {
    e.preventDefault();
    e.stopPropagation();
    if (n.isRest) return;
    openMenu(e, [
      !ancestors && n.dir && n.children.length && { label: "Entrer", icon: "layers", run: () => (owner === node ? enter(n) : enterKid(owner, n)) },
      { label: "Ouvrir", icon: "open", run: () => openPath(path).catch((er) => flash(String(er))) },
      { label: "Afficher dans l'Explorateur", icon: "folder", run: () => reveal(path, null).catch((er) => flash(String(er))) },
      "-",
      { label: "Mettre à la corbeille…", icon: "trash", danger: true, run: () => trash(n, owner, path, ancestors) },
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
      {#if tree && view === "blocks"}
        {#each stack as s, i}
          {#if i}<Icon name="chevron" size={13} />{/if}
          <button class:on={i === stack.length - 1} onclick={() => up(i)}>{i ? s.name : base}</button>
        {/each}
        <span class="tot">{fmtSize(node.size)} · {node.files.toLocaleString("fr-FR")} fichiers</span>
      {:else if tree}
        <b>Les plus lourds de {base}</b>
        <span class="tot">{fmtSize(tree.size)} · {tree.files.toLocaleString("fr-FR")} fichiers</span>
      {:else}
        <b>Analyse de {base}…</b>
        <span class="tot live">{progress.files.toLocaleString("fr-FR")} fichiers · {fmtSize(progress.bytes)}{used ? ` sur ${fmtSize(used)}` : ""} · {elapsed} s</span>
      {/if}
    </div>
    {#if tree}
      <div class="views">
        <button class:on={view === "blocks"} onclick={() => (view = "blocks")}><Icon name="grid" size={14} />Blocs</button>
        <button class:on={view === "list"} onclick={() => (view = "list")}><Icon name="list" size={14} />Liste</button>
      </div>
      <button class="hb" title="Analyser de nouveau" onclick={run}><Icon name="refresh" size={15} /></button>
    {/if}
    <button class="hb" title="Fermer (Échap)" onclick={close}><Icon name="close" size={16} /></button>
    {#if scanning && used}<div class="hbar"><span style="width:{pct}%"></span></div>{/if}
  </header>

  {#if err}
    <div class="state"><Icon name="alert" size={30} /><p>{err}</p></div>
  {:else if view === "list" && tree}
    <div class="list">
      {#each heavy as h, i (h.path)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="row" role="button" tabindex="-1" onclick={() => goTo(h)} oncontextmenu={(e) => menu(e, h.n, h.owner, h.path, h.chain)} title="Voir dans les blocs">
          <span class="rank">{i + 1}</span>
          <span class="rico" style="--c:{color(h.n)}"><Icon name={h.n.dir ? "folder" : "file"} size={16} /></span>
          <div class="rt">
            <b>{h.info.title}{#if h.info.tag}<i class="rtag" style="--c:{color(h.n)}">{h.info.tag}</i>{/if}</b>
            <small>{h.path}</small>
          </div>
          <div class="rs">
            <b>{fmtSize(h.n.size)}</b>
            <div class="rbar"><span style="width:{(h.n.size / heavy[0].n.size) * 100}%;--c:{color(h.n)}"></span></div>
          </div>
          <div class="racts">
            <button title="Afficher dans l'Explorateur" onclick={(e) => (e.stopPropagation(), reveal(h.path, null).catch((er) => flash(String(er))))}><Icon name="folder" size={15} /></button>
            <button class="del" title="Mettre à la corbeille" onclick={(e) => (e.stopPropagation(), trash(h.n, h.owner, h.path, h.chain))}><Icon name="trash" size={15} /></button>
          </div>
        </div>
      {/each}
    </div>
  {:else if node}
    {#if !scanning && stack.length > 1}
      <div class="mapbar" transition:slide={{ duration: 160 }}>
        <button class="back" title="Dossier précédent (Retour arrière)" onclick={() => up(stack.length - 2)}>
          <Icon name="chevron" size={15} />Retour à {stack.length > 2 ? stack[stack.length - 2].name : base}
        </button>
      </div>
    {/if}
    <div class="map" class:scanning bind:clientWidth={W} bind:clientHeight={H}>
      <div class="inner" bind:this={inner}>
        {#each blocks as b (b.path)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="blk"
            class:dir={b.n.dir && !b.n.isRest}
            class:rest={b.n.isRest}
            class:pending={b.n.pending}
            class:live={b.n.live}
            class:nested={b.kids.length}
            in:scale={{ start: 0.6, duration: 260 }}
            style="left:{b.x}px;top:{b.y}px;width:{b.w}px;height:{b.h}px;--c:{color(b.n)};--cs:{color(b.n, 0.16)}"
            role="button"
            tabindex="-1"
            onclick={() => enter(b.n, b)}
            oncontextmenu={(e) => !scanning && menu(e, b.n, node, b.path)}
            onmouseenter={() => (hover = b)}
            onmouseleave={() => hover === b && (hover = null)}
          >
            {#if b.w > 46 && b.h > 20}
              <div class="lbl" class:small={b.h < 44 || b.w < 90}>
                {#if b.n.pending}<span class="mini-spin"></span>{:else if b.n.dir && !b.n.isRest && b.w > 90}<Icon name="folder" size={12} />{/if}
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
                onclick={(e) => (e.stopPropagation(), enterKid(b.n, k.n, { x: b.x + k.x, y: b.y + k.y, w: k.w, h: k.h, parent: b }))}
                oncontextmenu={(e) => !scanning && menu(e, k.n, b.n, kp)}
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
        {#if hover}<b>{hover.path}</b> · {fmtSize(hover.n.size)}{#if hover.n.files > 1} · {hover.n.files.toLocaleString("fr-FR")} fichiers{/if}{:else if scanning}Les dossiers apparaissent dès qu'ils sont analysés…{:else}Clic : entrer · clic droit : ouvrir, supprimer · Retour arrière : remonter{/if}
      </span>
    </footer>
  {/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 90; background: var(--scrim); backdrop-filter: blur(3px); }
  .win { position: fixed; z-index: 91; inset: 4vh 3vw; display: flex; flex-direction: column; border-radius: 18px; overflow: hidden; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); }
  header { position: relative; display: flex; align-items: center; gap: 10px; padding: 12px 14px 12px 18px; border-bottom: 1px solid var(--line); }
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
  .state p { margin: 0; font-size: 14px; font-variant-numeric: tabular-nums; }

  .mapbar { display: flex; padding: 10px 10px 0; }
  .back { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 14px 0 10px; border-radius: 10px; border: 0; font: inherit; font-size: 13px; font-weight: 650; background: var(--accent); color: white; cursor: pointer; flex: none; box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 40%, transparent); transition: transform 0.12s; }
  .back :global(svg) { transform: rotate(180deg); }
  .back:hover { transform: translateX(-2px); }
  .tot.live { font-variant-numeric: tabular-nums; }
  .views { display: inline-flex; padding: 3px; border-radius: 10px; background: var(--chip); flex: none; }
  .views button { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; }
  .views button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }
  .hbar { position: absolute; left: 0; right: 0; bottom: -1px; height: 3px; background: var(--chip); }
  .hbar span { display: block; height: 100%; background: var(--accent); transition: width 0.3s; }
  .inner { position: absolute; inset: 0; transform-origin: 0 0; }
  .blk.pending { background: repeating-linear-gradient(135deg, hsl(220 12% 60% / 0.18) 0 10px, hsl(220 12% 60% / 0.08) 10px 20px); background-size: 28px 28px; animation: slide 1.2s linear infinite; color: var(--muted); cursor: default; border-style: dashed; border-color: var(--line-strong); }
  @keyframes slide { to { background-position: 28px 0; } }
  .blk.live { animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { filter: brightness(1.15) saturate(1.2); } }
  .map.scanning .blk { transition: left 0.35s ease, top 0.35s ease, width 0.35s ease, height 0.35s ease; cursor: progress; }
  .mini-spin { width: 11px; height: 11px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .list { flex: 1; overflow-y: auto; padding: 10px 14px 16px; display: flex; flex-direction: column; gap: 4px; }
  .row { display: flex; align-items: center; gap: 14px; padding: 9px 12px; border-radius: 12px; cursor: pointer; }
  .row:hover { background: var(--surface); box-shadow: var(--shadow-sm); }
  .rank { width: 22px; text-align: right; font-size: 12px; font-weight: 700; color: var(--muted); font-variant-numeric: tabular-nums; flex: none; }
  .rico { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; background: color-mix(in srgb, var(--c) 18%, transparent); color: var(--c); flex: none; }
  .rt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rt b { display: flex; align-items: center; gap: 8px; font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .rtag { padding: 1px 7px; border-radius: 5px; font-style: normal; font-size: 10.5px; font-weight: 700; background: color-mix(in srgb, var(--c) 16%, transparent); color: var(--c); }
  .rt small { font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .rs { width: 180px; flex: none; display: flex; flex-direction: column; align-items: flex-end; gap: 5px; }
  .rs b { font-size: 14px; font-variant-numeric: tabular-nums; }
  .rbar { width: 100%; height: 6px; border-radius: 6px; background: var(--chip); overflow: hidden; }
  .rbar span { display: block; height: 100%; border-radius: 6px; background: var(--c); }
  .racts { display: flex; gap: 4px; opacity: 0; transition: opacity 0.12s; }
  .row:hover .racts { opacity: 1; }
  .racts button { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; border: 0; background: var(--chip); color: var(--text-2); cursor: pointer; }
  .racts button:hover { color: var(--accent); }
  .racts button.del:hover { color: var(--err); background: var(--err-soft); }
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
