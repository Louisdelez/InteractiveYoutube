// Koala TV — web remote.
// Single WebSocket to the desktop app's embedded HTTP server.

(function () {
  "use strict";

  // ── Token extraction ──────────────────────────────────────────────
  const params = new URLSearchParams(location.search);
  const token = params.get("t");
  if (!token) {
    document.body.innerHTML = '<div style="padding:40px;color:#fff;font-family:sans-serif"><h2>Jeton manquant</h2><p>Rescanne le QR code depuis l\'app desktop.</p></div>';
    return;
  }

  // ── UI refs ───────────────────────────────────────────────────────
  const $grid = document.getElementById("grid");
  const $dots = document.getElementById("dots");
  const $current = document.getElementById("current-channel");
  const $volume = document.getElementById("volume");
  const $volValue = document.getElementById("volume-value");
  const $btnMute = document.getElementById("btn-mute");
  const $btnVolDown = document.getElementById("btn-vol-down");
  const $btnVolUp = document.getElementById("btn-vol-up");
  const $btnPrev = document.getElementById("btn-prev");
  const $btnPlay = document.getElementById("btn-play");
  const $btnNext = document.getElementById("btn-next");
  const $status = document.getElementById("status-bar");

  // ── State ─────────────────────────────────────────────────────────
  let state = null;
  let page = 0;
  const PAGE_SIZE = 9;

  // ── Haptic feedback ───────────────────────────────────────────────
  const vibrate = (ms) => {
    if (navigator.vibrate) navigator.vibrate(ms);
  };

  // ── WebSocket ─────────────────────────────────────────────────────
  let ws = null;
  let wsRetry = 0;
  let volumeDragging = false;

  function wsOpen() {
    const url = new URL("/ws", location.origin);
    url.searchParams.set("t", token);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    try {
      ws = new WebSocket(url.toString());
    } catch (e) {
      console.error("ws open error", e);
      scheduleReconnect();
      return;
    }
    ws.onopen = () => {
      wsRetry = 0;
      setStatus(true);
    };
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg.type === "state") onState(msg);
      } catch (e) {
        console.warn("bad ws msg", e);
      }
    };
    ws.onerror = () => {};
    ws.onclose = () => {
      setStatus(false);
      scheduleReconnect();
    };
  }

  function scheduleReconnect() {
    wsRetry = Math.min(wsRetry + 1, 5);
    const delay = Math.min(200 * Math.pow(2, wsRetry), 4000);
    setTimeout(wsOpen, delay);
  }

  function send(cmd) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(cmd));
    }
  }

  function setStatus(online) {
    $status.classList.toggle("offline", !online);
    $status.textContent = online ? "" : "Hors ligne — reconnexion…";
  }

  // ── State → UI ────────────────────────────────────────────────────
  function onState(s) {
    state = s;
    // Don't clobber the slider while the user is dragging it — the
    // desktop echo of their own drag would fight the UI.
    if (!volumeDragging) {
      const v = s.muted ? 0 : s.volume;
      $volume.value = String(v);
      $volValue.textContent = String(v);
    }
    $current.textContent = s.current_channel ? s.current_channel.name : "—";
    renderGrid();
  }

  function renderGrid() {
    const favs = (state && state.favorites) || [];
    const totalPages = Math.max(1, Math.ceil(favs.length / PAGE_SIZE));
    if (page >= totalPages) page = totalPages - 1;
    if (page < 0) page = 0;
    const start = page * PAGE_SIZE;
    const slice = favs.slice(start, start + PAGE_SIZE);
    const currentId = state && state.current_channel ? state.current_channel.id : null;

    const cells = [];
    for (let i = 0; i < PAGE_SIZE; i++) {
      const ch = slice[i];
      if (ch) {
        const active = ch.id === currentId ? " current" : "";
        const num = start + i + 1;
        cells.push(
          `<button class="cell${active}" data-id="${escapeAttr(ch.id)}" aria-label="${escapeAttr(ch.name)}">`
          + `<img src="/avatar/${encodeURIComponent(ch.id)}?t=${encodeURIComponent(token)}" alt="" onerror="this.remove()" />`
          + `<span class="num">${num}</span>`
          + `</button>`
        );
      } else {
        cells.push(`<div class="cell empty"></div>`);
      }
    }
    $grid.innerHTML = cells.join("");

    // Dots
    const dots = [];
    for (let p = 0; p < totalPages; p++) {
      dots.push(`<div class="dot${p === page ? " active" : ""}"></div>`);
    }
    $dots.innerHTML = dots.join("");
  }

  function escapeAttr(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;",
    })[c]);
  }

  // ── Grid tap + swipe ──────────────────────────────────────────────
  $grid.addEventListener("click", (e) => {
    const btn = e.target.closest(".cell[data-id]");
    if (!btn) return;
    vibrate(15);
    send({ cmd: "select_channel", id: btn.dataset.id });
  });

  // Swipe paging with Pointer Events (works on iOS Safari + Android).
  (function swipe() {
    let startX = 0;
    let startY = 0;
    let active = false;
    const THRESHOLD = 50;
    $grid.addEventListener("pointerdown", (e) => {
      active = true;
      startX = e.clientX;
      startY = e.clientY;
    }, { passive: true });
    $grid.addEventListener("pointerup", (e) => {
      if (!active) return;
      active = false;
      const dx = e.clientX - startX;
      const dy = e.clientY - startY;
      if (Math.abs(dx) > THRESHOLD && Math.abs(dx) > Math.abs(dy)) {
        const totalPages = Math.max(1, Math.ceil(((state && state.favorites) || []).length / PAGE_SIZE));
        if (dx < 0 && page < totalPages - 1) { page += 1; vibrate(10); renderGrid(); }
        else if (dx > 0 && page > 0) { page -= 1; vibrate(10); renderGrid(); }
      }
    }, { passive: true });
    $grid.addEventListener("pointercancel", () => { active = false; });
  })();

  // ── Volume slider ─────────────────────────────────────────────────
  let lastVolSent = 0;
  $volume.addEventListener("pointerdown", () => { volumeDragging = true; });
  $volume.addEventListener("pointerup", () => { volumeDragging = false; });
  $volume.addEventListener("pointercancel", () => { volumeDragging = false; });
  $volume.addEventListener("input", () => {
    const v = parseInt($volume.value, 10);
    $volValue.textContent = String(v);
    const now = Date.now();
    if (now - lastVolSent >= 80) {
      lastVolSent = now;
      send({ cmd: "set_volume", value: v });
    }
  });
  $volume.addEventListener("change", () => {
    send({ cmd: "set_volume", value: parseInt($volume.value, 10) });
  });

  // ── Buttons ───────────────────────────────────────────────────────
  const tap = (btn, cmd) => {
    btn.addEventListener("click", () => {
      vibrate(15);
      send(cmd);
    });
  };
  tap($btnMute, { cmd: "toggle_mute" });
  tap($btnVolDown, { cmd: "volume_down" });
  tap($btnVolUp, { cmd: "volume_up" });
  tap($btnPrev, { cmd: "prev_memory" });
  tap($btnPlay, { cmd: "force_play" });
  tap($btnNext, { cmd: "next_channel" });

  // ── Boot ──────────────────────────────────────────────────────────
  wsOpen();

  // Handle page visibility : reconnect on foreground.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && (!ws || ws.readyState !== WebSocket.OPEN)) {
      wsOpen();
    }
  });
})();
