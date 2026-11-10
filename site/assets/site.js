// Vyoma's site: the first run's glass lens over the dunes, springy presses, entrances, the
// install command picker and the status bubble. Nothing runs while the page is at rest.
(() => {
  "use strict";
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = (s, el = document) => el.querySelector(s);

  // ---------- Refraction maps ----------
  // A displacement map for a rounded rect: flat in the middle, and over the rim (a quarter circle,
  // about a fifth of the shape) it samples further and further inward, so the edge shows what's
  // across the glass, bent like water. Red = x, green = y, 128 = no shift.
  function rimMap(w, h, radius, rim) {
    const c = document.createElement("canvas");
    c.width = w; c.height = h;
    const ctx = c.getContext("2d");
    const img = ctx.createImageData(w, h);
    const hx = w / 2, hy = h / 2;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        // Signed distance to the rounded rect and its outward normal.
        const px = x + 0.5 - hx, py = y + 0.5 - hy;
        const qx = Math.abs(px) - (hx - radius), qy = Math.abs(py) - (hy - radius);
        let d, nx, ny;
        if (qx > 0 && qy > 0) {
          const l = Math.hypot(qx, qy) || 1;
          d = l - radius; nx = qx / l; ny = qy / l;
        } else if (qx > qy) {
          d = qx - radius; nx = 1; ny = 0;
        } else {
          d = qy - radius; nx = 0; ny = 1;
        }
        nx *= Math.sign(px) || 1; ny *= Math.sign(py) || 1;
        const i = (y * w + x) * 4;
        let ox = 0, oy = 0;
        if (d < 0) {
          const t = Math.min(Math.max(1 + d / rim, 0), 1);
          // A quarter circle, eased off at the very edge so the bend stays smooth.
          const u = t * 0.94, f = (1 - Math.sqrt(1 - u * u)) / (1 - Math.sqrt(1 - 0.94 * 0.94));
          ox = -nx * f; oy = -ny * f;
        }
        img.data[i] = 128 + ox * 127;
        img.data[i + 1] = 128 + oy * 127;
        img.data[i + 2] = 128;
        img.data[i + 3] = 255;
      }
    }
    ctx.putImageData(img, 0, 0);
    return c.toDataURL();
  }

  function setMap(filterId, mapId, w, h, radius, rim, strength) {
    const map = document.getElementById(mapId);
    const disp = document.getElementById(filterId).querySelector("feDisplacementMap");
    map.setAttribute("href", rimMap(Math.round(w), Math.round(h), radius, rim));
    map.setAttribute("width", Math.round(w));
    map.setAttribute("height", Math.round(h));
    disp.setAttribute("scale", strength);
  }

  // ---------- The lens ----------
  // As in the app's first run: a drop of glass to drag over "Vyoma". The scene (dunes and word) is
  // painted once into a bitmap; while the drop moves, each frame samples it through a curved rim,
  // smoothly (bilinear): magnified in the middle, and towards the edge it looks further and further
  // across, like a thick drop of water. Before the bitmap is ready it's just clear glass.
  const hero = $("#top"), scene = $("#scene"), lens = $("#lens"), hint = $("#hint");
  if (hero && lens) {
    const canvas = document.createElement("canvas");
    canvas.className = "lens-canvas";
    $("#lens-view").appendChild(canvas);
    const ctx = canvas.getContext("2d");
    const ZOOM = 1.12, DPR = Math.min(window.devicePixelRatio || 1, 2);
    let W = 0, H = 0, D = 0, x = 0, y = 0, vx = 0, vy = 0, tx = 0, ty = 0;
    let held = false, gx = 0, gy = 0, running = false, placed = false, moved = false;
    // It floats in: a small drop rising from the sand that swells to full size over the word.
    let restX = 0, restY = 0, entering = !reduced, sc = reduced ? 1 : 0.22, vs = 0;
    let src = null, sw = 0, sh = 0, table = null, L = 0, out = null, build = 0;

    // Where each lens pixel looks, in device px from the lens centre, and how much of it is inside.
    function lensTable() {
      L = Math.round(D * DPR);
      canvas.width = canvas.height = L;
      out = ctx.createImageData(L, L);
      const R = L / 2, rim = R * 0.32, K = R * 0.75;
      table = new Float32Array(L * L * 3);
      for (let j = 0; j < L; j++) {
        for (let i = 0; i < L; i++) {
          const px = i + 0.5 - R, py = j + 0.5 - R, r = Math.hypot(px, py), k = (j * L + i) * 3;
          let s = r / ZOOM;
          const e = R - r;
          if (e < rim) {
            const t = 1 - Math.max(e, 0) / rim;
            s -= (1 - Math.sqrt(1 - t * t * 0.97)) * K;
          }
          const n = r > 0 ? s / r : 1 / ZOOM;
          table[k] = px * n;
          table[k + 1] = py * n;
          table[k + 2] = Math.min(Math.max(R - r + 0.5, 0), 1);
        }
      }
    }

    async function paintSource() {
      const id = ++build;
      src = null;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      try {
        await document.fonts.load("800 100px Inter");
        const w = Math.round(W * DPR), h = Math.round(H * DPR);
        const c = document.createElement("canvas");
        c.width = w; c.height = h;
        const g = c.getContext("2d", { willReadFrequently: true });
        const svg = scene.querySelector("svg").cloneNode(true);
        svg.setAttribute("xmlns", "http://www.w3.org/2000/svg");
        svg.setAttribute("width", w); svg.setAttribute("height", h);
        const url = URL.createObjectURL(new Blob([new XMLSerializer().serializeToString(svg)], { type: "image/svg+xml" }));
        try {
          const img = new Image();
          img.src = url;
          await img.decode();
          if (id !== build) return;
          g.drawImage(img, 0, 0, w, h);
        } finally { URL.revokeObjectURL(url); }
        // The word, drawn exactly where the page draws it, shadows and all.
        const word = $(".word", scene), cs = getComputedStyle(word);
        const range = document.createRange();
        range.selectNodeContents(word);
        const rr = range.getBoundingClientRect(), hr = hero.getBoundingClientRect();
        g.scale(DPR, DPR);
        g.font = `800 ${cs.fontSize} Inter`;
        if ("letterSpacing" in g) g.letterSpacing = cs.letterSpacing;
        g.fillStyle = "#fff";
        g.textBaseline = "alphabetic";
        const base = rr.top - hr.top + g.measureText(word.textContent).fontBoundingBoxAscent;
        for (const [oy, blur, a] of [[8, 40, .25], [2, 4, .18]]) {
          g.shadowColor = `rgba(20, 40, 80, ${a})`;
          g.shadowOffsetY = oy * DPR;
          g.shadowBlur = blur * DPR;
          g.fillText(word.textContent, rr.left - hr.left, base);
        }
        if (id !== build) return;
        src = g.getImageData(0, 0, w, h).data;
        sw = w; sh = h;
        paint();
      } catch (e) {
        src = null; // stays clear glass
      }
    }

    function paint() {
      if (!src) return;
      const cx = (x + D / 2) * DPR, cy = (y + D / 2) * DPR;
      const s = src, o = out.data, mx = sw - 1.001, my = sh - 1.001, row = sw * 4;
      for (let p = 0, k = 0, q = 0, n = L * L; p < n; p++, k += 3, q += 4) {
        const cov = table[k + 2];
        if (cov === 0) { o[q + 3] = 0; continue; }
        let fx = cx + table[k], fy = cy + table[k + 1];
        fx = fx < 0 ? 0 : fx > mx ? mx : fx;
        fy = fy < 0 ? 0 : fy > my ? my : fy;
        const x0 = fx | 0, y0 = fy | 0, ax = fx - x0, ay = fy - y0;
        const a = (y0 * sw + x0) * 4, b = a + 4, c = a + row, d = c + 4;
        for (let ch = 0; ch < 3; ch++) {
          const t = s[a + ch] + (s[b + ch] - s[a + ch]) * ax;
          const u = s[c + ch] + (s[d + ch] - s[c + ch]) * ax;
          o[q + ch] = t + (u - t) * ay;
        }
        o[q + 3] = 255 * cov;
      }
      ctx.putImageData(out, 0, 0);
    }

    function measure() {
      W = hero.clientWidth; H = hero.clientHeight; D = lens.offsetWidth;
      if (!placed || entering) {
        const range = document.createRange();
        range.selectNodeContents($(".word", scene));
        const r = range.getBoundingClientRect(), h = hero.getBoundingClientRect();
        restX = Math.min(Math.max(r.left - h.left - D * 0.12, 0), W - D);
        restY = Math.min(Math.max(r.top - h.top + r.height / 2 - D / 2, 0), H - D);
        if (!placed) {
          x = restX; y = restY;
          if (entering) { x = restX + Math.min(W * 0.22, D * 1.4); y = Math.min(restY + H * 0.34, H - D); }
          placed = true;
        }
      }
      x = Math.min(Math.max(x, 0), W - D);
      y = Math.min(Math.max(y, 0), H - D);
      lensTable();
      draw();
      paintSource();
    }

    function draw() {
      const s = Math.hypot(vx, vy), k = Math.min(s / 45, 1) * 0.1, a = Math.atan2(vy, vx);
      lens.style.transform = `translate(${x}px, ${y}px) rotate(${a}rad) scale(${sc * (1 + k)}, ${sc * (1 - k * 0.7)}) rotate(${-a}rad)`;
      lens.style.opacity = Math.min(Math.max((sc - 0.22) / 0.4, 0), 1);
      // The sun is the light: the rim is brightest where it faces it.
      const fit = Math.max(W / 1600, H / 1000);
      const sx = (W - 1600 * fit) / 2 + 930 * fit, sy = (H - 1000 * fit) / 2 + 500 * fit;
      const deg = Math.atan2(sx - (x + D / 2), -(sy - (y + D / 2))) * 180 / Math.PI;
      lens.style.setProperty("--la", deg + "deg");
      paint();
    }

    function step() {
      // Swelling to full size wobbles out like a drop (underdamped), held or not.
      vs = (vs + (1 - sc) * 0.035) * 0.8;
      sc += vs;
      if (entering && !held) {
        vx = (vx + (restX - x) * 0.014) * 0.85;
        vy = (vy + (restY - y) * 0.014) * 0.85;
        x += vx; y += vy;
        if (Math.hypot(restX - x, restY - y) < 0.3 && Math.hypot(vx, vy) < 0.05 && Math.abs(1 - sc) < 0.002) {
          entering = false;
          if (!moved) hint.classList.remove("gone");
        }
      } else if (held) {
        const nx = x + (tx - x) * 0.32, ny = y + (ty - y) * 0.32;
        vx = nx - x; vy = ny - y; x = nx; y = ny;
      } else {
        x += vx; y += vy;
        vx *= 0.955; vy *= 0.955;
        if (x < 0) { x = 0; vx = -vx * 0.6; }
        if (x > W - D) { x = W - D; vx = -vx * 0.6; }
        if (y < 0) { y = 0; vy = -vy * 0.6; }
        if (y > H - D) { y = H - D; vy = -vy * 0.6; }
      }
      const settling = Math.abs(1 - sc) > 0.002 || Math.abs(vs) > 0.001;
      if (held || entering || settling || Math.hypot(vx, vy) > 0.04) { draw(); requestAnimationFrame(step); }
      else { vx = vy = 0; sc = 1; vs = 0; draw(); running = false; }
    }
    const run = () => { if (!running) { running = true; requestAnimationFrame(step); } };

    lens.addEventListener("pointerdown", e => {
      held = true;
      entering = false;
      lens.classList.add("held");
      lens.setPointerCapture(e.pointerId);
      const h = hero.getBoundingClientRect();
      gx = e.clientX - h.left - x; gy = e.clientY - h.top - y;
      tx = x; ty = y;
      run();
    });
    lens.addEventListener("pointermove", e => {
      if (!held) return;
      const h = hero.getBoundingClientRect();
      tx = Math.min(Math.max(e.clientX - h.left - gx, 0), W - D);
      ty = Math.min(Math.max(e.clientY - h.top - gy, 0), H - D);
      if (!moved) { moved = true; hint.classList.add("gone"); }
    });
    const release = () => {
      if (!held) return;
      held = false;
      lens.classList.remove("held");
      if (reduced) { vx = vy = 0; }
    };
    lens.addEventListener("pointerup", release);
    lens.addEventListener("pointercancel", release);

    let resizeTimer = 0, lastW = 0;
    addEventListener("resize", () => {
      // Phones resize on every scroll as the address bar comes and goes: only a new width repaints.
      if (hero.clientWidth === lastW && Math.abs(hero.clientHeight - H) < 120) return;
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => { lastW = hero.clientWidth; measure(); }, 150);
    });
    lastW = hero.clientWidth;
    if (entering) hint.classList.add("gone");
    measure();
    if (entering) setTimeout(run, 450);
  }

  // ---------- Nav: clear over the dunes, refracting where the browser can ----------
  const nav = $("#nav");
  if (nav && hero && "IntersectionObserver" in window) {
    new IntersectionObserver(([e]) => nav.classList.toggle("over-hero", e.isIntersecting), { rootMargin: "-68px 0px 0px 0px" }).observe(hero);
  }
  const chromium = navigator.userAgentData && navigator.userAgentData.brands.some(b => /Chromium/.test(b.brand));
  if (nav && chromium && !reduced) {
    const refract = () => {
      const w = nav.offsetWidth, h = nav.offsetHeight;
      setMap("nav-fx", "nav-map", w, h, h / 2, h * 0.3, h * 0.5);
    };
    refract();
    nav.classList.add("refract");
    let t = 0;
    addEventListener("resize", () => { clearTimeout(t); t = setTimeout(refract, 150); });
  }

  // ---------- Press: every edge moves 3 px in; it springs back on release ----------
  document.addEventListener("pointerdown", e => {
    const el = e.target.closest("[data-press]");
    if (!el) return;
    const w = el.offsetWidth, h = el.offsetHeight;
    el.classList.add("pressed");
    el.style.transform = `scale(${(w - 6) / w}, ${(h - 6) / h})`;
    const up = () => {
      el.classList.remove("pressed");
      el.style.transform = "";
      removeEventListener("pointerup", up);
      removeEventListener("pointercancel", up);
      el.removeEventListener("pointerleave", up);
    };
    addEventListener("pointerup", up);
    addEventListener("pointercancel", up);
    el.addEventListener("pointerleave", up);
  });

  // ---------- Entrances: rise in once, 40 ms apart ----------
  const rising = document.querySelectorAll(".rise");
  if (!reduced && "IntersectionObserver" in window) {
    const io = new IntersectionObserver(entries => {
      let n = 0;
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        e.target.style.setProperty("--delay", (n++ * 40) + "ms");
        e.target.classList.add("in");
        io.unobserve(e.target);
      }
    }, { threshold: 0.1, rootMargin: "0px 0px -6% 0px" });
    rising.forEach(el => io.observe(el));
  } else {
    rising.forEach(el => el.classList.add("in"));
  }

  // ---------- Status bubble ----------
  const bubble = $("#bubble");
  let bubbleTimer = 0;
  function status(text) {
    bubble.querySelector("span").textContent = text;
    bubble.classList.add("show");
    clearTimeout(bubbleTimer);
    bubbleTimer = setTimeout(() => bubble.classList.remove("show"), 1600);
  }

  // ---------- Install command: curl or wget ----------
  const seg = $("#seg"), cmd = $("#cmd code");
  if (seg) {
    seg.querySelectorAll("button").forEach((b, i) => b.addEventListener("click", () => {
      seg.dataset.at = i;
      seg.querySelectorAll("button").forEach(o => o.setAttribute("aria-selected", String(o === b)));
      cmd.textContent = b.dataset.cmd;
    }));
  }

  document.querySelectorAll("[data-copy]").forEach(btn => btn.addEventListener("click", async () => {
    const text = $(btn.dataset.copy).textContent;
    try {
      await navigator.clipboard.writeText(text);
      status("Copied. Paste it into a terminal");
    } catch {
      const r = document.createRange();
      r.selectNodeContents($(btn.dataset.copy));
      const s = getSelection(); s.removeAllRanges(); s.addRange(r);
      status("Selected. Press Ctrl+C to copy");
    }
  }));
})();
