<script>
  // Studio d'extraits : lire une vidéo locale (x265, multi-pistes, sous-titres), choisir des passages
  // (chacun avec sa piste audio et ses sous-titres) et les exporter en un MP4 partageable.
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { untrack } from "svelte";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import Wave from "./Wave.svelte";
  import { clipAsset, clipExport, clipOpen, clipProxy, clipWave } from "./api.js";
  import { confirmAsk, flash, ui } from "./ui.svelte.js";

  /** path : vidéo (ou dossier) ; onexported() : export lancé */
  let { path, onexported } = $props();

  let src = $state(null);
  let err = $state("");
  $effect(() => {
    clipOpen(path)
      .then((s) => {
        src = s;
        audio = (s.audio.find((a) => a.default) ?? s.audio[0])?.n ?? null;
        end = Math.min(60, s.duration);
        height = Math.min(1080, s.height || 1080);
      })
      .catch((e) => (err = String(e)));
  });

  // ---------- lecture ----------
  // Deux modes : « direct » (flux converti à la volée depuis l'instant voulu : chaque saut redémarre la conversion)
  // puis « rapide » dès que la copie 360p est prête (fichier normal : sauts instantanés, sous-titres affichés par le lecteur).
  let video = $state();
  let base = $state(0); // direct : instant de départ du flux
  let rel = $state(0); // temps dans le flux / le fichier
  let paused = $state(true);
  let loading = $state(true);
  let audio = $state(null);
  let sub = $state(null);
  let nonce = $state(0);
  let stopAt = null;
  let proxy = $state(null);
  $effect(() => {
    if (!src) return;
    const id = setInterval(async () => {
      proxy = await clipProxy(src.id).catch(() => null);
      if (proxy?.ready || proxy?.error) clearInterval(id);
    }, 1000);
    return () => clearInterval(id);
  });
  const imageSub = $derived(sub != null && !!src?.subs.find((s) => s.n === sub)?.image);
  // Les sous-titres en images (Blu-ray) ne passent que par le flux direct
  const fast = $derived(!!proxy?.ready && !imageSub);
  const cur = $derived(fast ? rel : base + rel);
  const D = $derived(src?.duration || 1);
  const url = $derived(
    !src ? "" : fast ? `${proxy.base}p${audio ?? "none"}.mp4` : `${src.stream}?t=${base.toFixed(2)}&a=${audio ?? -1}&s=${sub ?? -1}&h=540&k=${nonce}`,
  );
  // Passage direct ↔ rapide : on garde l'instant
  let lastCur = 0;
  let pending = null;
  let wasFast = false;
  $effect(() => {
    const f = fast;
    if (f === wasFast) return;
    wasFast = f;
    const t = lastCur;
    wantPlay = !untrack(() => paused);
    loading = true;
    if (f) {
      pending = t;
      base = 0;
      rel = t;
    } else {
      base = t;
      rel = 0;
      nonce++;
    }
  });

  let wantPlay = false;
  let seekTimer;
  function seek(t, play = !paused) {
    t = Math.min(Math.max(0, t), Math.max(0, D - 0.5));
    if (fast && video) {
      video.currentTime = t;
      rel = lastCur = t;
      if (play) video.play().catch(() => {});
      return;
    }
    // Direct : déjà reçu → saut immédiat
    if (video && t >= base) {
      const r = t - base;
      const b = video.buffered;
      for (let i = 0; i < b.length; i++) {
        if (r >= b.start(i) && r <= b.end(i) - 0.3) {
          video.currentTime = r;
          rel = r;
          if (play) video.play().catch(() => {});
          return;
        }
      }
    }
    clearTimeout(seekTimer);
    base = t;
    rel = 0;
    loading = true;
    wantPlay = play;
    // Regroupe les sauts rapprochés (glisser la timeline)
    seekTimer = setTimeout(() => nonce++, 120);
  }
  function onloaded() {
    loading = false;
    if (pending != null && video) {
      video.currentTime = pending;
      rel = pending;
      pending = null;
    }
    showSubs();
    if (wantPlay) video?.play().catch(() => {});
  }
  // Piste de sous-titres du mode rapide : visible tout de suite
  function showSubs() {
    if (!video) return;
    for (const t of video.textTracks) t.mode = "showing";
  }
  function toggle() {
    if (!video) return;
    if (video.paused) {
      stopAt = null;
      video.play().catch(() => {});
    } else video.pause();
  }
  function ontime() {
    rel = video.currentTime;
    lastCur = cur;
    syncMusic();
    if (stopAt != null && cur >= stopAt) {
      video.pause();
      stopAt = null;
    }
  }
  // Changer de piste : on reprend au même instant
  function setTracks(a, s) {
    const t = cur;
    const audioChanged = a !== audio;
    audio = a;
    sub = s;
    if (fast && !imageSub) {
      // Rapide : changer d'audio = autre fichier (même instant) ; sous-titres = simple piste du lecteur
      if (audioChanged) {
        pending = t;
        wantPlay = !paused;
        loading = true;
      } else setTimeout(showSubs, 50);
      return;
    }
    seek(t);
  }

  // ---------- sélection ----------
  let start = $state(0);
  let end = $state(60);
  let track;
  let drag = null;
  // Zoom : portion visible [va, vb] de la vidéo (molette = zoom autour du curseur, Maj+molette = défiler)
  let va = $state(0);
  let vb = $state(60);
  let fitted = false;
  $effect(() => {
    if (src && !fitted) {
      fitted = true;
      va = 0;
      vb = src.duration || 60;
    }
  });
  const span = $derived(Math.max(0.5, vb - va));
  const pct = (t) => `${((t - va) / span) * 100}%`;
  const wpct = (d) => `${(d / span) * 100}%`;
  function zoom(factor, center = (va + vb) / 2) {
    const ns = Math.min(Math.max(span * factor, 2), D);
    let a = center - (center - va) * (ns / span);
    a = Math.min(Math.max(0, a), Math.max(0, D - ns));
    va = a;
    vb = a + ns;
  }
  function pan(dt) {
    const a = Math.min(Math.max(0, va + dt), Math.max(0, D - span));
    vb = a + span;
    va = a;
  }
  function onwheel(e) {
    e.preventDefault();
    if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) pan(((e.deltaX || e.deltaY) / (track?.clientWidth || 800)) * span);
    else zoom(e.deltaY > 0 ? 1.25 : 0.8, at(e));
  }
  // La tête de lecture sort de la vue pendant la lecture : la vue suit
  $effect(() => {
    if (!paused && (cur > vb || cur < va) && span < D) pan(cur - va - span * 0.1);
  });
  const fitAll = () => ((va = 0), (vb = D));
  const fitSel = () => {
    const pad = Math.max(1, (end - start) * 0.15);
    va = Math.max(0, start - pad);
    vb = Math.min(D, end + pad);
  };

  // Courbe du son de la piste choisie (une fois par piste)
  let waves = $state({});
  $effect(() => {
    if (!src || audio == null || waves[audio]) return;
    const n = audio;
    waves[n] = [];
    clipWave(src.id, n)
      .then((p) => (waves[n] = p))
      .catch(() => {});
  });
  const peaks = $derived(audio != null ? waves[audio] : null);
  const two = (n) => String(n).padStart(2, "0");
  function fmt(t, tenth = false) {
    t = Math.max(0, t || 0);
    const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = Math.floor(t % 60);
    const b = h ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
    return tenth ? `${b}.${Math.floor((t % 1) * 10)}` : b;
  }
  const parse = (txt) => {
    const p = String(txt).trim().split(/[:h.,m]/).filter((x) => x !== "");
    if (!p.length || p.some((x) => !/^\d+$/.test(x))) return NaN;
    return p.reduce((s, x) => s * 60 + +x, 0);
  };
  function at(e) {
    const r = track.getBoundingClientRect();
    return va + Math.min(Math.max(0, (e.clientX - r.left) / r.width), 1) * span;
  }
  let dragT = $state(null);
  function down(e, what) {
    e.preventDefault();
    e.stopPropagation();
    drag = what;
    track.setPointerCapture(e.pointerId);
    move(e);
  }
  function move(e) {
    if (!drag) return;
    const t = Math.round(at(e) * 10) / 10;
    if (drag === "start") start = Math.min(t, end - 0.5);
    else if (drag === "end") end = Math.max(t, start + 0.5);
    else dragT = t;
  }
  function up() {
    if (!drag) return;
    if (drag === "start") seek(start, false);
    else if (drag === "end") seek(Math.max(0, end - 2), false);
    else if (dragT != null) seek(dragT);
    drag = null;
    dragT = null;
  }
  const setIn = () => (start = Math.min(cur, end - 0.5));
  const setOut = () => (end = Math.max(cur, start + 0.5));
  function playSel() {
    seek(start, true);
    stopAt = end;
  }

  let fromTxt = $state("0:00");
  let toTxt = $state("1:00");
  $effect(() => {
    fromTxt = fmt(start, true);
  });
  $effect(() => {
    toTxt = fmt(end, true);
  });
  function commit(which) {
    const v = parse(which === "start" ? fromTxt : toTxt);
    if (!Number.isFinite(v)) return which === "start" ? (fromTxt = fmt(start, true)) : (toTxt = fmt(end, true));
    if (which === "start") start = Math.min(Math.max(0, v), end - 0.5);
    else end = Math.max(Math.min(D, v), start + 0.5);
    seek(which === "start" ? start : Math.max(0, end - 2), false);
  }

  // ---------- calques (texte / image) et musiques ----------
  let layers = $state([]);
  let music = $state([]);
  let volume = $state(1); // son de la vidéo
  let picked = $state(null); // calque ou musique sélectionné (id)
  let uid = 0;
  const pickedLayer = $derived(layers.find((l) => l.id === picked));
  const pickedMusic = $derived(music.find((m) => m.id === picked));
  const COLORS = ["#ffffff", "#ffd84d", "#ff5a5a", "#4cc2ff", "#7dff8a", "#111111"];

  function addText() {
    const l = { id: ++uid, kind: "text", start, end, x: 0.5, y: 0.82, size: 0.07, text: "Ton texte", color: "#ffffff", boxed: true };
    layers = [...layers, l];
    picked = l.id;
  }
  async function addImage() {
    const p = await pickFile({ title: "Image à ajouter", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] }] });
    if (typeof p !== "string") return;
    try {
      const a = await clipAsset(p);
      const l = { id: ++uid, kind: "image", start, end, x: 0.82, y: 0.2, size: 0.25, path: p, url: a.url, name: a.name };
      layers = [...layers, l];
      picked = l.id;
    } catch (e) {
      flash(String(e));
    }
  }
  async function addMusic() {
    const p = await pickFile({ title: "Musique à ajouter", filters: [{ name: "Sons", extensions: ["mp3", "m4a", "aac", "wav", "ogg", "opus", "flac", "mp4", "webm", "mkv"] }] });
    if (typeof p !== "string") return;
    try {
      const a = await clipAsset(p);
      if (a.kind !== "audio") return flash("Ce fichier n'a pas de son");
      const m = { id: ++uid, path: p, url: a.url, name: a.name, start, offset: 0, total: a.duration, duration: Math.min(a.duration || end - start, D - start), volume: 0.8 };
      music = [...music, m];
      picked = m.id;
    } catch (e) {
      flash(String(e));
    }
  }
  function removePicked() {
    layers = layers.filter((l) => l.id !== picked);
    music = music.filter((m) => m.id !== picked);
    picked = null;
  }
  // Calé sur la sélection actuelle
  function fitPicked() {
    if (pickedLayer) Object.assign(pickedLayer, { start, end });
    if (pickedMusic) Object.assign(pickedMusic, { start, duration: Math.min(end - start, (pickedMusic.total || 1e9) - pickedMusic.offset) });
  }

  // Glisser un bloc sur sa piste (déplacer, ou tirer un bord)
  let bdrag = null;
  function bdown(e, item, mode) {
    e.preventDefault();
    e.stopPropagation();
    picked = item.id;
    bdrag = { item, mode, t0: at(e), s0: item.start, e0: item.end, d0: item.duration, o0: item.offset };
    lanes.setPointerCapture(e.pointerId);
  }
  function bmove(e) {
    if (!bdrag) return;
    const { item, mode, t0, s0, e0, d0, o0 } = bdrag;
    const dt = Math.round((at(e) - t0) * 10) / 10;
    if (item.kind) {
      const len = e0 - s0;
      if (mode === "move") {
        item.start = Math.min(Math.max(0, s0 + dt), D - len);
        item.end = item.start + len;
      } else if (mode === "l") item.start = Math.min(Math.max(0, s0 + dt), item.end - 0.3);
      else item.end = Math.max(Math.min(D, e0 + dt), item.start + 0.3);
    } else if (mode === "move") item.start = Math.min(Math.max(0, s0 + dt), D - 0.3);
    else if (mode === "r") item.duration = Math.max(0.3, Math.min(d0 + dt, (item.total || 1e9) - item.offset, D - item.start));
    else {
      const d = Math.min(Math.max(dt, -o0, -s0), d0 - 0.3);
      item.start = s0 + d;
      item.offset = o0 + d;
      item.duration = d0 - d;
    }
  }
  const bup = () => (bdrag = null);
  // Molette : écouteur non passif (sinon impossible d'empêcher la page de défiler pendant le zoom)
  $effect(() => {
    const el = lanes;
    if (!el) return;
    el.addEventListener("wheel", onwheel, { passive: false });
    return () => el.removeEventListener("wheel", onwheel);
  });
  let lanes = $state();

  // Placer un calque en le glissant sur la vidéo
  let sw = $state(0);
  let sh = $state(0);
  const stage = $derived.by(() => {
    const ar = src?.width && src?.height ? src.width / src.height : 16 / 9;
    let w = sw, h = sw / ar;
    if (h > sh) ((h = sh), (w = sh * ar));
    return { w, h, l: (sw - w) / 2, t: (sh - h) / 2 };
  });
  let stageEl;
  let sdrag = null;
  function sdown(e, l) {
    e.preventDefault();
    e.stopPropagation();
    picked = l.id;
    sdrag = l;
    stageEl.setPointerCapture(e.pointerId);
  }
  function smove(e) {
    if (!sdrag) return;
    const r = stageEl.getBoundingClientRect();
    sdrag.x = Math.min(Math.max(0, (e.clientX - r.left) / r.width), 1);
    sdrag.y = Math.min(Math.max(0, (e.clientY - r.top) / r.height), 1);
  }
  const visible = (l) => (cur >= l.start && cur <= l.end) || l.id === picked;

  // Aperçu des musiques : calées sur la vidéo
  let players = {};
  function syncMusic(force = false) {
    for (const m of music) {
      const el = players[m.id];
      if (!el) continue;
      el.volume = Math.min(1, m.volume);
      const inside = cur >= m.start && cur < m.start + m.duration;
      if (inside && !paused) {
        const want = m.offset + (cur - m.start);
        if (force || Math.abs(el.currentTime - want) > 0.35) el.currentTime = want;
        if (el.paused) el.play().catch(() => {});
      } else if (!el.paused) el.pause();
    }
  }
  $effect(() => {
    paused;
    music.length;
    untrack(() => syncMusic(true));
  });
  $effect(() => {
    if (video) video.volume = Math.min(1, volume);
  });

  // ---------- pistes ----------
  const LANG = { fre: "Français", fra: "Français", fr: "Français", jpn: "Japonais", ja: "Japonais", eng: "Anglais", en: "Anglais", spa: "Espagnol", ger: "Allemand", deu: "Allemand", ita: "Italien", kor: "Coréen", chi: "Chinois", zho: "Chinois", por: "Portugais", ara: "Arabe", rus: "Russe" };
  const audioName = (a) => {
    const t = a.title && !/^(stereo|surround|aac|ac3|dts|5\.1|2\.0)/i.test(a.title) ? a.title : LANG[a.lang] || a.lang?.toUpperCase() || `Piste ${a.n + 1}`;
    const ch = a.channels > 2 ? ` · ${a.channels === 6 ? "5.1" : a.channels === 8 ? "7.1" : a.channels + " can."}` : "";
    return `${t}${ch}`;
  };
  const subName = (s) => (s.title || LANG[s.lang] || s.lang?.toUpperCase() || `Piste ${s.n + 1}`) + (s.forced && !/forc/i.test(s.title) ? " (forcés)" : "");
  const short = (n, list, name) => (n == null ? null : name(list.find((x) => x.n === n) ?? {}));

  // ---------- montage ----------
  let segs = $state([]);
  function addSeg() {
    if (end - start < 0.5) return flash("Passage trop court");
    segs = [...segs, { start, end, audio, sub }];
    flash(`Passage ${segs.length} ajouté au montage`);
  }
  function previewSeg(s) {
    audio = s.audio;
    sub = s.sub;
    start = s.start;
    end = s.end;
    playSel();
  }
  function moveSeg(i, d) {
    const j = i + d;
    if (j < 0 || j >= segs.length) return;
    const c = [...segs];
    [c[i], c[j]] = [c[j], c[i]];
    segs = c;
  }
  const total = $derived((segs.length ? segs : [{ start, end }]).reduce((s, x) => s + (x.end - x.start), 0));

  // ---------- export ----------
  let height = $state(1080);
  let name = $state("");
  let busy = $state(false);
  const HEIGHTS = $derived([2160, 1440, 1080, 720, 480].filter((h) => h <= (src?.height || 1080)));
  // Estimation grossière (H.264 CRF 20) : ~ débit selon la définition
  const RATE = { 2160: 16, 1440: 9, 1080: 5.5, 720: 3, 480: 1.6 };
  const est = $derived(Math.round(((RATE[height] ?? 4) + 0.19) * total / 8 * 10) / 10);

  async function exportNow() {
    const list = segs.length ? segs : [{ start, end, audio, sub }];
    busy = true;
    try {
      await clipExport(
        src.path,
        list.map((s) => ({ start: s.start, end: s.end, audio: s.audio, sub: s.sub })),
        height,
        name || null,
        layers.map(({ kind, start, end, x, y, size, text = "", color = "#ffffff", boxed = false, path = "" }) => ({ kind, start, end, x, y, size, text, color, boxed, path })),
        music.map(({ path, start, offset, duration, volume }) => ({ path, start, offset, duration, volume })),
        volume,
      );
      flash("Export lancé — suis-le dans l'onglet DL");
      onexported?.();
      ui.studio = null;
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }
  async function close() {
    if (segs.length && !(await confirmAsk("Fermer le studio ?", `Les ${segs.length} passages du montage seront perdus.`, "Fermer", true))) return;
    ui.studio = null;
  }

  function key(e) {
    if (ui.confirm || e.target.tagName === "INPUT" || e.target.tagName === "SELECT" || e.target.tagName === "TEXTAREA") return;
    if ((e.key === "Delete" || e.key === "Backspace") && picked != null) return removePicked();
    if (e.key === "Escape") close();
    else if (e.key === " ") (e.preventDefault(), toggle());
    else if (e.key === "ArrowLeft") seek(cur - (e.shiftKey ? 1 : 5));
    else if (e.key === "ArrowRight") seek(cur + (e.shiftKey ? 1 : 5));
    else if (e.key.toLowerCase() === "i") setIn();
    else if (e.key.toLowerCase() === "o") setOut();
    else if (e.key === "Enter") addSeg();
  }
</script>

<svelte:window onkeydown={key} />

<div class="backdrop" transition:fade={{ duration: 120 }} role="presentation"></div>
<div class="studio" transition:scale={{ start: 0.98, duration: 150 }} role="dialog" aria-label="Studio d'extraits">
  <header>
    <span class="logo"><Icon name="scissors" size={16} /></span>
    <div class="ttl">
      <b>Studio d'extraits</b>
      <span title={src?.path}>{src?.name ?? "Ouverture…"}</span>
    </div>
    {#if src}
      {#if fast}
        <span class="chip fastc" title="Copie légère prête : sauts instantanés dans la timeline (l'export garde la qualité d'origine)"><Icon name="check" size={11} />Lecture rapide</span>
      {:else if proxy && !proxy.ready && !proxy.error && !imageSub}
        <span class="chip" title="Une copie légère se prépare en fond ; ensuite, les sauts dans la timeline seront instantanés">Lecture rapide · {Math.round(proxy.progress * 100)} %</span>
      {/if}
      <span class="chip">{src.width}×{src.height} · {src.vcodec.toUpperCase()}{src.hdr ? " · HDR" : ""}</span>
    {/if}
    <button class="x" title="Fermer (Échap)" onclick={close}><Icon name="close" size={15} /></button>
  </header>

  {#if err}
    <p class="err"><Icon name="alert" size={15} />{err}</p>
  {:else if !src}
    <p class="wait"><span class="spin"></span>Lecture des pistes…</p>
  {:else}
    <div class="body">
      <div class="left">
        <div class="screen" bind:clientWidth={sw} bind:clientHeight={sh}>
          {#key nonce}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={video} src={url} crossorigin="anonymous" preload="auto" bind:paused ontimeupdate={ontime} onloadeddata={onloaded} onseeked={() => (loading = false)} onwaiting={() => (loading = true)} onplaying={() => (loading = false)} onclick={toggle}>
              {#if fast && sub != null}
                {#key sub}<track kind="subtitles" src="{proxy.base}s{sub}.vtt" srclang="fr" label="Sous-titres" default onload={showSubs} />{/key}
              {/if}
            </video>
          {/key}
          {#if loading}<span class="spin big"></span>{/if}
          {#if paused && !loading}<button class="bigplay" onclick={toggle} aria-label="Lecture"><Icon name="play" size={26} /></button>{/if}
          <!-- Calques : aperçu fidèle, à glisser pour les placer -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="stage" bind:this={stageEl} style="left:{stage.l}px;top:{stage.t}px;width:{stage.w}px;height:{stage.h}px" onpointermove={smove} onpointerup={() => (sdrag = null)} onpointercancel={() => (sdrag = null)}>
            {#each layers as l (l.id)}
              {#if visible(l)}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="lay"
                  class:picked={l.id === picked}
                  class:ghost={!(cur >= l.start && cur <= l.end)}
                  style="left:{l.x * 100}%;top:{l.y * 100}%"
                  onpointerdown={(e) => sdown(e, l)}
                >
                  {#if l.kind === "image"}
                    <img src={l.url} alt="" style="width:{l.size * stage.w}px" draggable="false" />
                  {:else}
                    <span class="txt" class:boxed={l.boxed} style="font-size:{l.size * stage.h}px;color:{l.color}">{l.text}</span>
                  {/if}
                </div>
              {/if}
            {/each}
          </div>
        </div>
        {#each music as m (m.id)}<audio bind:this={players[m.id]} src={m.url} preload="auto"></audio>{/each}

        <div class="ctl">
          <button class="ib" onclick={toggle} title="Lecture / pause (espace)"><Icon name={paused ? "play" : "pause"} size={15} /></button>
          <button class="ib t" onclick={() => seek(cur - 5)} title="−5 s (←, Maj+← : −1 s)">−5</button>
          <button class="ib t" onclick={() => seek(cur + 5)} title="+5 s (→, Maj+→ : +1 s)">+5</button>
          <span class="now">{fmt(dragT ?? cur, true)} <i>/ {fmt(D)}</i></span>
          <span class="sp"></span>
          <button class="mini" onclick={setIn} title="Raccourci : I"><b>[</b> Début ici</button>
          <button class="mini" onclick={setOut} title="Raccourci : O">Fin ici <b>]</b></button>
          <button class="mini" onclick={playSel}><Icon name="play" size={11} />Lire la sélection</button>
        </div>

        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="tl" bind:this={lanes} onpointermove={bmove} onpointerup={bup} onpointercancel={bup}>
          <div class="lane" title="Textes et images (par-dessus la vidéo)">
            <span class="ltag"><Icon name="image" size={11} /></span>
            {#each layers as l (l.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="blk lay-b" class:on={l.id === picked} style="left:{pct(l.start)};width:{wpct(l.end - l.start)}" onpointerdown={(e) => bdown(e, l, "move")}>
                <span class="eg l" onpointerdown={(e) => bdown(e, l, "l")}></span>
                <span class="bl">{l.kind === "image" ? l.name : l.text}</span>
                <span class="eg r" onpointerdown={(e) => bdown(e, l, "r")}></span>
              </div>
            {/each}
          </div>
          <div class="track" bind:this={track} onpointerdown={(e) => down(e, "seek")} onpointermove={move} onpointerup={up} onpointercancel={up}>
          <Wave {peaks} perSec={50} from={va} to={vb} />
          {#each segs as s, i}<div class="mark" style="left:{pct(s.start)};width:{wpct(s.end - s.start)}" title="Passage {i + 1}"></div>{/each}
          <div class="dim" style="left:0;width:{pct(start)}"></div>
          <div class="dim" style="left:{pct(end)};right:0"></div>
          <div class="sel" style="left:{pct(start)};width:{wpct(end - start)}"></div>
          <div class="head" style="left:{pct(dragT ?? cur)}"></div>
          <div class="h" style="left:{pct(start)}" onpointerdown={(e) => down(e, "start")} role="slider" aria-label="Début" aria-valuenow={start} tabindex="-1"><span></span></div>
          <div class="h" style="left:{pct(end)}" onpointerdown={(e) => down(e, "end")} role="slider" aria-label="Fin" aria-valuenow={end} tabindex="-1"><span></span></div>
          </div>
          <div class="lane" title="Musiques (mixées avec le son de la vidéo)">
            <span class="ltag"><Icon name="audio" size={11} /></span>
            {#each music as m (m.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="blk mus-b" class:on={m.id === picked} style="left:{pct(m.start)};width:{wpct(m.duration)}" onpointerdown={(e) => bdown(e, m, "move")}>
                <span class="eg l" onpointerdown={(e) => bdown(e, m, "l")}></span>
                <span class="bl">♪ {m.name}</span>
                <span class="eg r" onpointerdown={(e) => bdown(e, m, "r")}></span>
              </div>
            {/each}
          </div>
          <div class="zrow">
            <span>{fmt(va)}</span>
            <span class="sp"></span>
            <button class="zb" onclick={() => zoom(0.5, cur)} title="Zoomer (molette sur la timeline)"><Icon name="plus" size={12} /></button>
            <button class="zb" onclick={() => zoom(2, cur)} title="Dézoomer"><b>−</b></button>
            <button class="zb t" onclick={fitSel} title="Cadrer la sélection">Sélection</button>
            <button class="zb t" onclick={fitAll} title="Toute la vidéo">Tout</button>
            <span class="sp"></span>
            <span>{fmt(vb)}</span>
          </div>
        </div>

        <div class="selrow">
          <label>De <input bind:value={fromTxt} onchange={() => commit("start")} spellcheck="false" /></label>
          <label>à <input bind:value={toTxt} onchange={() => commit("end")} spellcheck="false" /></label>
          <span class="len">{fmt(end - start, true)}</span>
          <span class="sp"></span>
          <button class="btn primary" onclick={addSeg} title="Entrée"><Icon name="plus" size={14} />Ajouter au montage</button>
        </div>
      </div>

      <aside>
        <section>
          <h4>Audio</h4>
          <div class="opts">
            {#each src.audio as a}
              <button class:on={audio === a.n} onclick={() => setTracks(a.n, sub)}>{audioName(a)}</button>
            {/each}
            {#if !src.audio.length}<span class="muted">Pas de son</span>{/if}
          </div>
          {#if src.audio.length}
            <label class="vol"><Icon name="audio" size={13} />Son de la vidéo <input type="range" min="0" max="2" step="0.05" bind:value={volume} /><b>{Math.round(volume * 100)} %</b></label>
          {/if}
        </section>
        <section>
          <h4>Sous-titres</h4>
          <div class="opts">
            <button class:on={sub == null} onclick={() => setTracks(audio, null)}>Aucun</button>
            {#each src.subs as s}
              <button class:on={sub === s.n} onclick={() => setTracks(audio, s.n)}>{subName(s)}</button>
            {/each}
          </div>
        </section>

        <section>
          <h4>Ajouter</h4>
          <div class="adds">
            <button onclick={addText}><Icon name="tag" size={13} />Texte</button>
            <button onclick={addImage}><Icon name="image" size={13} />Image</button>
            <button onclick={addMusic}><Icon name="audio" size={13} />Musique</button>
          </div>
          {#if pickedLayer || pickedMusic}
            <div class="edit">
              {#if pickedLayer?.kind === "text"}
                <textarea bind:value={pickedLayer.text} rows="2" spellcheck="false"></textarea>
                <label class="rng">Taille <input type="range" min="0.03" max="0.2" step="0.005" bind:value={pickedLayer.size} /></label>
                <div class="sw">
                  {#each COLORS as c}<button class:on={pickedLayer.color === c} style="background:{c}" title={c} aria-label="Couleur {c}" onclick={() => (pickedLayer.color = c)}></button>{/each}
                  <label class="chk"><input type="checkbox" bind:checked={pickedLayer.boxed} />Fond</label>
                </div>
              {:else if pickedLayer}
                <span class="en" title={pickedLayer.name}>{pickedLayer.name}</span>
                <label class="rng">Taille <input type="range" min="0.05" max="1" step="0.01" bind:value={pickedLayer.size} /></label>
              {:else if pickedMusic}
                <span class="en" title={pickedMusic.name}>♪ {pickedMusic.name}</span>
                <label class="rng">Volume <input type="range" min="0" max="2" step="0.05" bind:value={pickedMusic.volume} /><b>{Math.round(pickedMusic.volume * 100)} %</b></label>
                <span class="muted">Commence à {fmt(pickedMusic.offset, true)} dans la musique · tire le bord gauche du bloc pour en sauter le début</span>
              {/if}
              <div class="erow">
                <span class="muted">{fmt(pickedLayer?.start ?? pickedMusic.start)} → {fmt(pickedLayer?.end ?? pickedMusic.start + pickedMusic.duration)}</span>
                <span class="sp"></span>
                <button class="tiny w" onclick={fitPicked} title="Mettre sur la sélection actuelle">Caler</button>
                <button class="tiny" onclick={removePicked} title="Supprimer (Suppr)"><Icon name="trash" size={12} /></button>
              </div>
            </div>
          {:else}
            <p class="muted">Ils couvrent la sélection actuelle ; déplace-les sur leur piste, et glisse texte / image directement sur la vidéo.</p>
          {/if}
        </section>

        <section class="mont">
          <h4>Montage <span class="n">{segs.length}</span></h4>
          {#if !segs.length}
            <p class="muted">Choisis un passage puis « Ajouter au montage ». Ajoute-le plusieurs fois avec d'autres pistes (VF puis VOSTFR…) : ils s'enchaînent dans la vidéo finale. Sans montage, c'est la sélection actuelle qui est exportée.</p>
          {:else}
            <ol>
              {#each segs as s, i}
                <li>
                  <button class="seg" onclick={() => previewSeg(s)} title="Revoir ce passage">
                    <b>{i + 1}</b>
                    <span class="st">{fmt(s.start)} → {fmt(s.end)}</span>
                    <span class="tr">{short(s.audio, src.audio, audioName) ?? "Muet"}{s.sub != null ? ` · ST ${short(s.sub, src.subs, subName)}` : ""}</span>
                  </button>
                  <button class="tiny" onclick={() => moveSeg(i, -1)} disabled={i === 0} title="Monter"><Icon name="up" size={12} /></button>
                  <button class="tiny" onclick={() => moveSeg(i, 1)} disabled={i === segs.length - 1} title="Descendre"><Icon name="down" size={12} /></button>
                  <button class="tiny" onclick={() => (segs = segs.filter((_, j) => j !== i))} title="Retirer"><Icon name="close" size={12} /></button>
                </li>
              {/each}
            </ol>
          {/if}
        </section>

        <section class="exp">
          <div class="row">
            <label class="q">Qualité
              <select bind:value={height}>
                {#each HEIGHTS as h}<option value={h}>{h >= 2000 ? "4K" : `${h}p`}{h === Math.min(1080, src.height) ? " (recommandé)" : ""}</option>{/each}
              </select>
            </label>
            <span class="est">≈ {est.toString().replace(".", ",")} Mo · {fmt(total)}</span>
          </div>
          <input class="name" bind:value={name} placeholder="Nom du fichier (facultatif)" spellcheck="false" />
          <button class="btn primary go" disabled={busy} onclick={exportNow}>
            <Icon name="download" size={14} />{segs.length > 1 ? `Exporter le montage (${segs.length} passages)` : "Exporter l'extrait"}
          </button>
        </section>
      </aside>
    </div>
  {/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 140; background: rgb(0 0 0 / 0.6); }
  .studio {
    position: fixed; z-index: 141; inset: 3vh 3vw; display: flex; flex-direction: column;
    border-radius: 18px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); overflow: hidden;
  }
  header { display: flex; align-items: center; gap: 12px; padding: 12px 14px 12px 16px; border-bottom: 1px solid var(--line); }
  .logo { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; background: var(--accent-soft); color: var(--accent); }
  .ttl { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .ttl b { font-size: 14.5px; }
  .ttl span { font-size: 12px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .chip { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; font-weight: 700; padding: 3px 8px; border-radius: 6px; background: var(--chip); color: var(--text-2); font-variant-numeric: tabular-nums; }
  .chip.fastc { background: rgb(34 197 94 / 0.14); color: var(--ok); }
  .screen video::cue { font-size: 20px; background: rgb(0 0 0 / 0.55); }
  .x { display: grid; place-items: center; width: 32px; height: 32px; border: 0; border-radius: 9px; background: none; color: var(--muted); cursor: pointer; }
  .x:hover { background: var(--chip); color: var(--text); }
  .err, .wait { display: flex; align-items: center; gap: 10px; margin: auto; color: var(--muted); }
  .err { color: var(--err); }

  .body { flex: 1; min-height: 0; display: flex; }
  .left { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 10px; padding: 14px 16px 16px; }
  .screen { position: relative; flex: 1; min-height: 160px; border-radius: 12px; overflow: hidden; background: #000; display: grid; place-items: center; }
  .screen video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
  .bigplay { position: absolute; width: 64px; height: 64px; border-radius: 50%; border: 0; display: grid; place-items: center; padding-left: 4px; background: rgb(0 0 0 / 0.55); color: white; cursor: pointer; }
  .bigplay:hover { background: var(--accent); }
  .spin { width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  .spin.big { position: absolute; width: 38px; height: 38px; border-width: 3px; border-color: rgb(255 255 255 / 0.2); border-top-color: white; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .ctl, .selrow { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ib { display: grid; place-items: center; min-width: 34px; height: 34px; padding: 0 8px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .ib:hover { border-color: var(--accent); }
  .now { font-size: 13px; font-weight: 700; font-variant-numeric: tabular-nums; margin-left: 4px; }
  .now i { font-style: normal; color: var(--muted); font-weight: 500; }
  .sp { flex: 1; }
  .mini { display: inline-flex; align-items: center; gap: 5px; height: 32px; padding: 0 11px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .mini:hover { color: var(--text); border-color: var(--accent); }
  .mini b { color: var(--accent); }

  .tl { display: flex; flex-direction: column; gap: 4px; }
  .track { position: relative; height: 58px; margin: 0 8px; border-radius: 8px; background: var(--chip); cursor: pointer; touch-action: none; user-select: none; overflow: hidden; }
  .lane { position: relative; height: 24px; margin: 0 8px; border-radius: 6px; background: color-mix(in srgb, var(--chip) 55%, transparent); overflow: hidden; touch-action: none; user-select: none; }
  .ltag { position: absolute; left: 4px; top: 50%; translate: 0 -50%; color: var(--muted); opacity: 0.6; pointer-events: none; }
  .blk { position: absolute; top: 2px; bottom: 2px; min-width: 6px; display: flex; align-items: center; border-radius: 5px; cursor: grab; font-size: 11px; font-weight: 650; color: white; overflow: hidden; box-sizing: border-box; }
  .blk:active { cursor: grabbing; }
  .lay-b { background: #8b5cf6; }
  .mus-b { background: #10b981; }
  .blk.on { box-shadow: 0 0 0 2px var(--text); z-index: 2; }
  .bl { flex: 1; min-width: 0; padding: 0 6px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; pointer-events: none; }
  .eg { width: 7px; align-self: stretch; flex: none; cursor: ew-resize; background: rgb(255 255 255 / 0.25); }
  .zrow { display: flex; align-items: center; gap: 4px; margin: 0 8px; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .zb { display: inline-flex; align-items: center; justify-content: center; min-width: 26px; height: 22px; padding: 0 6px; border-radius: 6px; border: 1px solid var(--line); background: var(--surface); color: var(--text-2); font: inherit; font-size: 11px; font-weight: 650; cursor: pointer; }
  .zb:hover { border-color: var(--accent); color: var(--text); }
  .track { --wave: color-mix(in srgb, var(--accent) 55%, transparent); }
  .stage { position: absolute; pointer-events: none; }
  .lay { position: absolute; translate: -50% -50%; pointer-events: auto; cursor: move; line-height: 1.15; }
  .lay.picked { outline: 2px dashed rgb(255 255 255 / 0.8); outline-offset: 3px; }
  .lay.ghost { opacity: 0.45; }
  .lay img { display: block; }
  .txt { display: block; white-space: pre; text-align: center; font-family: Arial, sans-serif; font-weight: 800; text-shadow: 0 0 3px #000, 0 0 6px rgb(0 0 0 / 0.8); }
  .txt.boxed { text-shadow: none; background: rgb(0 0 0 / 0.55); padding: 0.18em 0.35em; }
  .vol, .rng { display: flex; align-items: center; gap: 8px; margin-top: 10px; font-size: 12.5px; color: var(--muted); }
  .vol input, .rng input { flex: 1; accent-color: var(--accent); }
  .vol b, .rng b { min-width: 42px; text-align: right; color: var(--text-2); font-variant-numeric: tabular-nums; }
  .adds { display: flex; gap: 6px; }
  .adds button { flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: 5px; height: 32px; border-radius: 8px; border: 1px dashed var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .adds button:hover { border-style: solid; border-color: var(--accent); color: var(--accent); }
  .edit { display: flex; flex-direction: column; gap: 4px; margin-top: 10px; padding: 10px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line); }
  .edit textarea { resize: vertical; min-height: 44px; padding: 6px 8px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--bg-2); color: var(--text); font: inherit; font-size: 13px; }
  .edit textarea:focus { outline: none; border-color: var(--accent); }
  .en { font-size: 12.5px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sw { display: flex; align-items: center; gap: 6px; margin-top: 8px; }
  .sw button { width: 20px; height: 20px; border-radius: 50%; border: 2px solid var(--line-strong); cursor: pointer; padding: 0; }
  .sw button.on { border-color: var(--accent); box-shadow: 0 0 0 2px var(--ring); }
  .chk { display: inline-flex; align-items: center; gap: 5px; margin-left: auto; font-size: 12.5px; color: var(--text-2); }
  .erow { display: flex; align-items: center; gap: 4px; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .tiny.w { width: auto; padding: 0 8px; font: inherit; font-size: 12px; font-weight: 650; border: 1px solid var(--line); }
  .mark { position: absolute; top: 0; bottom: 0; background: rgb(76 194 255 / 0.18); border-left: 1px solid rgb(76 194 255 / 0.5); border-right: 1px solid rgb(76 194 255 / 0.5); }
  .dim { position: absolute; top: 0; bottom: 0; background: rgb(0 0 0 / 0.18); }
  .sel { position: absolute; top: 0; bottom: 0; background: var(--accent-soft); border-top: 3px solid var(--accent); border-bottom: 3px solid var(--accent); box-sizing: border-box; }
  .head { position: absolute; top: -5px; bottom: -5px; width: 2px; margin-left: -1px; background: var(--text); pointer-events: none; }
  .h { position: absolute; top: -3px; bottom: -3px; width: 16px; margin-left: -8px; cursor: ew-resize; display: grid; place-items: center; }
  .h span { width: 11px; height: 100%; border-radius: 4px; background: var(--accent); box-shadow: 0 2px 6px rgb(0 0 0 / 0.3); }
  .selrow label { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted); }
  .selrow input, .name { height: 32px; padding: 0 8px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; }
  .selrow input { width: 82px; text-align: center; font-variant-numeric: tabular-nums; }
  .selrow input:focus, .name:focus, select:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .len { font-size: 12.5px; font-weight: 700; color: var(--accent); font-variant-numeric: tabular-nums; }

  aside { width: 330px; flex: none; display: flex; flex-direction: column; gap: 16px; padding: 14px 16px 16px; border-left: 1px solid var(--line); overflow: auto; }
  h4 { display: flex; align-items: center; gap: 6px; margin: 0 0 8px; font-size: 11.5px; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); }
  h4 .n { font-size: 10.5px; padding: 0 6px; border-radius: 6px; background: var(--chip); color: var(--text-2); letter-spacing: 0; }
  .opts { display: flex; flex-wrap: wrap; gap: 6px; }
  .opts button { height: 30px; padding: 0 11px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .opts button:hover { color: var(--text); border-color: var(--accent); }
  .opts button.on { background: var(--accent); border-color: var(--accent); color: white; }
  .muted { margin: 0; font-size: 12.5px; color: var(--muted); line-height: 1.5; }
  .mont { flex: 1; }
  ol { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  li { display: flex; align-items: center; gap: 3px; }
  .seg { flex: 1; min-width: 0; display: grid; grid-template-columns: auto 1fr; column-gap: 8px; padding: 6px 9px; border-radius: 9px; border: 1px solid var(--line); background: var(--surface); color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .seg:hover { border-color: var(--accent); }
  .seg b { grid-row: span 2; align-self: center; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; background: var(--accent-soft); color: var(--accent); font-size: 12px; }
  .seg .st { font-size: 12.5px; font-weight: 650; font-variant-numeric: tabular-nums; }
  .seg .tr { font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tiny { display: grid; place-items: center; width: 24px; height: 24px; padding: 0; border: 0; border-radius: 6px; background: none; color: var(--muted); cursor: pointer; }
  .tiny:hover:not(:disabled) { background: var(--chip); color: var(--text); }
  .tiny:disabled { opacity: 0.3; cursor: default; }
  .exp { display: flex; flex-direction: column; gap: 8px; padding-top: 12px; border-top: 1px solid var(--line); }
  .exp .row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .q { display: inline-flex; align-items: center; gap: 8px; font-size: 12.5px; color: var(--muted); }
  select { height: 32px; padding: 0 8px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12.5px; }
  .est { font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .go { width: 100%; justify-content: center; height: 38px; }

  @media (max-width: 900px) {
    .body { flex-direction: column; overflow: auto; }
    aside { width: auto; border-left: 0; border-top: 1px solid var(--line); }
    .screen { min-height: 240px; flex: none; aspect-ratio: 16 / 9; }
  }
</style>
