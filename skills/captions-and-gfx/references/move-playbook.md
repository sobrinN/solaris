# Solaris Programmatic GFX Move Playbook (10-Category Pattern Library)

Use this playbook whenever choosing or authoring a `G1` motion-graphic move—either as a **transparent overlay** on top of a talking-head video (`V1`) or as a **full-canvas programmatic scene** (`100vw × 100vh`) in a faceless explainer or zero-asset video.

## Placement Rules Before Building

1. **One Move Per Idea Worth Seeing**: Place a graphic only when the speaker states a concrete number, comparison, step, tool, quote, or turning point. Never stack two floating overlays within `4.0s` of each other.
2. **Instrument, Not a Sentence**: To show "fast", animate a counter or stopwatch stopping; "growth", a sparkline or bar race; "comparison", a 2×2 matrix or pros/cons split; "proof", a receipt, quote card, or KPI tile.
3. **Overlay vs Full-Canvas Cutaway**:
   - **Floating Overlay (over `V1` talking head)**: Transparent background (no full-screen backdrop/vignette), give cards enough opaque/frosted body (`rgba(26,14,14,0.88)` + `1px` rim) to read over any footage, enter from empty at `t=0`, exit cleanly in the last `0.30–0.40s`, and call `timeline.set_transform(track_id="g1", y=-0.22)` so it stays clear of the face and `T1` captions (`y: 0.20`).
   - **Full-Canvas Programmatic Scene (zero-asset or B-roll cutaway)**: Fill `100vw × 100vh` with layered ambient glow + subtle grid/grain, keep `Transform2D` at `(0, 0)`, keep all foreground cards/diagrams inside `5vh..61vh` on `9:16` (`6vh..72vh` on `16:9`) so `T1` captions never collide, and hold core visuals visible to the scene cut (no fade-to-black before exit).

---

## 1. The 10-Category Spoken-Cue → Visual-Move Taxonomy

Match what the script or speaker says to the exact visual archetype:

### A. Data & Proof (Numbers, Growth, Rankings, Comparisons)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Reels off 2–4 results ("revenue up 32%, users up 18%") | **`kpi-tiles`** | 2×2 (9:16) or 1×4 (16:9) glass tiles fly in from 3D depth → numbers count (`tabular-nums`) while SVG sparklines draw → hero tile lifts with accent rim & sheen |
| Compares options by one number ("X is 3× faster") | **`bar-race`** | Horizontal bars grow with `expo.out`, leader overtakes and stamps with accent halo |
| Growth or trend over time | **`line-reveal`** / **`line-cross`** | Smooth cubic SVG curve draws via `strokeDashoffset`, glowing dot rides tip, crossing point triggers pulse ring + value pop |
| Conversion drop-off across steps | **`funnel-stages`** | Tapered stepped bands reveal top-to-bottom with `%` counts and conversion deltas |
| Breaks total into parts / where time/money goes | **`stacked-bars`** / **`treemap`** / **`bubble-pack`** | Proportional blocks/circles scale in (`back.out(1.4)`), dominant share highlights in `--brand-accent` |
| Adds several numbers into one total | **`bracket-sum`** / **`waterfall-chart`** | Line items stagger in → SVG curly bracket draws → final sum counts & stamps |
| Ranks top 3 or overtakes #1 | **`podium-rank`** / **`leaderboard`** | Rows slide/reorder by `y` transform; winner row elevates with rim glow |
| "High impact, low effort" / beats trade-offs | **`matrix-2x2`** / **`scatter-quadrant`** / **`radar-chart`** | Orthogonal axes draw → competitor dots pop → hero dot lands in target quadrant with halo |
| "Only 3% do this" / "37 out of 100" | **`distribution-curve`** / **`unit-dots`** | Bell curve tail fills in accent, or 10×10 dot grid lights up exact count sequentially |
| Streak, duration, or overload | **`day-counter`** / **`contribution-heatmap`** / **`gauge-needle`** | Flip/slot day count, GitHub-style activity grid wave, or SVG arc needle sweeping into red/accent zone |
| Retention or multi-channel comparison | **`cohort-grid`** / **`sparkline-list`** / **`slope-chart`** / **`tier-pyramid`** | Color-coded retention cells, inline mini-sparklines, before→after slope lines, or stacked pyramid tiers |

### B. Explain & Frameworks (Breaking Ideas Into Parts)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Sums up what a product/system includes | **`bento-reveal`** | Asymmetric 3–5 tile bento grid springs in (`0.06s` stagger), each tile runs a tiny live SVG micro-viz |
| Breaks concept into N parts / architecture | **`explainer-build`** / **`exploded-ui`** | Single card splits/expands into labeled layers with orthogonal SVG leader lines |
| Lists N steps, names them as one system | **`step-bracket-label`** / **`timeline-track`** | Nodes light up sequentially along an SVG rail, bracket groups them under the unified label |
| Weighs trade-offs or gives a verdict | **`pros-cons`** / **`verdict-card`** | Two-column split (semantic green checks vs red crosses), or bold verdict card stamped with `APPROVED` / `SKIP` |
| "What you see is only the tip" | **`iceberg-reveal`** | Waterline bisects frame; surface label sits above, camera/rig pans down to reveal hidden layers below |
| "One thing becomes many" / connected parts | **`mind-map`** / **`chain-links`** | Center node bursts SVG bezier branches to child pills, or interlocking chain links snap together |
| "Everyone does X... except one" / holds step back | **`odd-one-out-highlight`** / **`locked-list-reveal`** | Grid of muted cards dims while 1 hero card lifts; or numbered list shows blurred/locked row that unlocks with stamp |
| Calls out a detail / focuses attention | **`spotlight-dim`** / **`magnifier-loupe`** / **`label-box-callout`** | Darkened backdrop dims surroundings while circular mask or loupe ring zooms/frames the target detail |
| Adjusts one lever/setting | **`rate-dial-knob`** | Rotary SVG dial rotates smoothly while linked output metric counts in real time |

### C. AI & Automation (Agents, Prompts, Pipelines)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Multi-stage workflow ("brief → research → edit") | **`agent-pipeline`** / **`tool-map`** | Connected glass nodes along glowing SVG rail; active pulse travels node-to-node lighting each stage |
| "Just type this prompt..." / AI streams answer | **`ask-bar`** / **`ai-stream`** / **`prompt-template-copy`** | Command bar types query (`28 cps`), cursor clicks Send, response lines stream in with subtle caret |
| Multiple agents working in parallel / hub | **`agents-board`** / **`agent-hub`** | Central core with radial SVG data paths firing into 3–5 concurrent status cards |
| "It cites sources / checks before acting" | **`cited-answer`** / **`agent-approval`** / **`tool-call-trace`** | Answer chips pop inline `[1] [2]`, or trace log steps tick green sequentially ending in Approve button press |
| Context limits / model switching / automations | **`context-meter`** / **`model-picker`** / **`automation-rule`** / **`connector-toggle`** | Segmented fill bar approaching limit, dropdown selection click, `IF → THEN` rule bridge, or switch toggles snapping green |

### D. Dev & Engineering (Code, Terminal, Shipping)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| "One command to install/deploy" | **`terminal-run`** | Dark monospace window, command types out, progress bar fills, semantic green `✓ Ready` line lands |
| "AI rewrote the function" / fixed a bug | **`code-diff`** / **`error-to-fix`** / **`review-suggestion`** | Red `-` lines strike/collapse while green `+` lines wipe in; red error badge flips to green `Tests Passed` |
| Speedup / API call / deploy / PR merge | **`network-waterfall`** / **`api-call`** / **`deploy-live`** / **`pr-merge`** | Waterfall timing bars shrink dramatically (`4.8s → 0.3s`), JSON request→`200 OK` badge, or purple→green merged pill |
| Branches, logs, status, SQL, gradual rollout | **`git-graph`** / **`log-filter`** / **`status-page`** / **`sql-query`** / **`rollout-slider`** | SVG commit rail merging, log rows filtering to 1 highlighted match, uptime bars turning green, or `1% → 100%` slider |

### E. Social Proof & Creator Economy
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Retells a text/DM conversation | **`chat-thread`** / **`dm-inbox`** / **`slack-message`** | Staggered left/right message bubbles pop in (`back.out(1.8)`) with realistic timestamps |
| Reads viewer comment / viral post / CTA | **`comment-card`** / **`comment-keyword`** / **`post-card`** / **`tweet-thread`** | Author avatar circle + quote + heart/reply counters; keyword pill types and highlights |
| Audience poll, ratings, testimonials | **`community-poll`** / **`review-stars`** / **`testimonial-wall`** | Animated poll bars racing to `%`, 5 SVG stars filling sequentially, or staggered 2×2 quote cards |
| Growth / retention / CTR / listicle number | **`reel-insights`** / **`retention-graph`** / **`thumb-ctr-row`** / **`listicle-number`** / **`chapter-tag`** | Metric tiles + retention curve highlight, giant ordinal badge (`03`), or scrub-bar chapter indicator |
| Subscribe / Follow / Lead magnet | **`follow-card`** / **`subscribe-button`** / **`lead-magnet-cover`** | Cursor clicks Follow/Subscribe → button flips to `Following ✓` with subtle particle ring |

### F. Business, Productivity, Editorial Paper & System UI
| Category | High-Impact Archetypes | Core Visual Mechanic |
| --- | --- | --- |
| **Business & Commerce** | `pricing-toggle`, `price-tag` (strike old → reveal new), `invoice-paid`, `payment-success`, `crm-deal-card`, `money-flow-path`, `coupon-code-badge`, `order-tracker`, `waitlist-spot` | Monthly→Yearly toggle flip, semantic green `PAID` stamp, animated SVG flow arrows between wallet/bank/revenue nodes |
| **Productivity & Workflow** | `checklist`, `step-tracker`, `kanban-move`, `calendar-week`, `inbox-zero`, `spotlight-search`, `spreadsheet-fill`, `timer-stopwatch`, `meeting-notes` | Sequential green SVG checkmarks (`strokeDashoffset`), card sliding column-to-column, crisp `⌘K` search bar typing |
| **Editorial & Paper** | `headline-highlight`, `strike-correct`, `stamp-approve`, `receipt-print`, `report-card`, `sticky-note`, `index-card-flip`, `blueprint`, `quote-card`, `paper-tear` | Yellow/accent highlighter wipe behind text, red diagonal strike + replacement word slam, thermal receipt scrolling up, 3D `rotateY(180deg)` card flip |
| **OS & System UI** | `notify-stack`, `keycap-shortcut`, `control-toggles`, `software-update`, `storage-bar`, `battery-charge`, `window-tile`, `tabs-multiply`, `permission-dialog`, `location-pin-chip` | 3 stacked frosted notifications popping in, tactile 3D keycap depression (`y: +4px` + shadow shrink), switches flipping green |

---

## 2. Production-Ready Solaris HTML/CSS/GSAP Blueprints

Every blueprint below is **100% Solaris-native**, passes `gfx.lint_html` out of the box, uses `--brand-*` tokens, `window.gsap`, `window.__timelines["main"]`, responsive units, and safe vertical bands (`5vh..61vh` on 9:16). Adapt copy, numbers (only sourced!), and layout to the active scene.

### Blueprint 1: 3D Glass KPI Tiles (Sparklines + Count-Up + Hero Lift)
Best for: proving 2–4 real sourced metrics on the drop or payoff beat.

```html
<div class="clip" data-start="0" data-duration="4.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);font-family:var(--brand-font-body,Sora),sans-serif;color:var(--brand-ink,#F7F5EF);">
  <style>
    .amb{position:absolute;inset:-20%;pointer-events:none}
    .amb i{position:absolute;border-radius:50%;filter:blur(90px);opacity:.38;background:radial-gradient(circle,var(--brand-accent,#F05A00) 0%,transparent 68%)}
    .stage{position:absolute;left:6vw;right:6vw;top:7vh;height:52vh;perspective:1600px;display:flex;flex-direction:column;justify-content:center;gap:2.2vh}
    .eyebrow{font-size:clamp(12px,2.4vmin,18px);font-weight:700;letter-spacing:.18em;text-transform:uppercase;color:var(--brand-accent-secondary,#F7B733)}
    .grid{display:grid;grid-template-columns:repeat(2,1fr);gap:2.4vmin;transform-style:preserve-3d}
    .tile{position:relative;padding:2.6vmin;border-radius:22px;overflow:hidden;
      background:linear-gradient(155deg,rgba(255,255,255,.15),rgba(255,255,255,.04));
      -webkit-backdrop-filter:blur(20px) saturate(150%);backdrop-filter:blur(20px) saturate(150%);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.42),inset 0 0 0 1px rgba(255,255,255,.12),0 28px 60px -24px rgba(0,0,0,.65)}
    .tile::after{content:"";position:absolute;inset:0;pointer-events:none;mix-blend-mode:screen;
      background:linear-gradient(110deg,transparent calc(var(--sheen,-45%) - 18%),rgba(255,255,255,.28) var(--sheen,-45%),transparent calc(var(--sheen,-45%) + 18%))}
    .tile.hero{box-shadow:inset 0 1px 0 rgba(255,255,255,.55),inset 0 0 0 2px color-mix(in srgb,var(--brand-accent,#F05A00) 75%,white),0 28px 70px -20px color-mix(in srgb,var(--brand-accent,#F05A00) 45%,transparent)}
    .top{display:flex;justify-content:space-between;align-items:center;margin-bottom:1.2vmin}
    .lbl{font-size:clamp(12px,2.5vmin,18px);font-weight:600;opacity:.72}
    .delta{font-size:clamp(11px,2.1vmin,15px);font-weight:700;padding:.3em .65em;border-radius:999px;color:#2BD67B;background:rgba(43,214,123,.16);font-variant-numeric:tabular-nums}
    .num{font-family:var(--brand-font-display,Sora),sans-serif;font-size:clamp(26px,6.4vmin,56px);font-weight:800;letter-spacing:-0.025em;line-height:1.05;font-variant-numeric:tabular-nums}
    .sp{width:100%;height:5.5vh;margin-top:1.2vmin;overflow:visible}
    .sp path{fill:none;stroke:var(--brand-accent-secondary,#F7B733);stroke-width:3;stroke-linecap:round}
  </style>
  <div class="amb"><i style="width:65vw;height:45vh;left:35%;top:2%"></i><i style="width:70vw;height:45vh;left:-25%;top:55%;opacity:.26"></i></div>
  <div class="stage" id="rig">
    <div class="eyebrow" id="ey">02 / 05 · VERIFIED RESULTS</div>
    <div class="grid">
      <div class="tile" id="t0"><div class="top"><span class="lbl">Revenue</span><span class="delta">+32%</span></div><div class="num" id="n0"></div><svg class="sp" viewBox="0 0 200 48"><path id="p0" d="M4,40 C40,36 72,30 110,22 C145,15 170,12 196,6"/></svg></div>
      <div class="tile" id="t1"><div class="top"><span class="lbl">Retention</span><span class="delta">+18%</span></div><div class="num" id="n1"></div><svg class="sp" viewBox="0 0 200 48"><path id="p1" d="M4,38 C45,35 80,28 120,24 C155,20 175,14 196,10"/></svg></div>
    </div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#ey", { opacity: 0, filter: "blur(6px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.28, ease: "power2.out" }, 0);
    const items = [
      { id: "#t0", n: "#n0", p: "#p0", to: 48.2, pre: "$", post: "K", dec: 1 },
      { id: "#t1", n: "#n1", p: "#p1", to: 94, pre: "", post: "%", dec: 0 }
    ];
    items.forEach((it, i) => {
      const at = 0.08 + i * 0.14;
      tl.fromTo(it.id, { y: 48, z: -260, rotationX: 16, opacity: 0 }, { y: 0, z: 0, rotationX: 0, opacity: 1, duration: 0.65, ease: "expo.out" }, at);
      tl.fromTo(it.id, { "--sheen": "-45%" }, { "--sheen": "145%", duration: 0.8, ease: "power2.inOut", immediateRender: false }, at + 0.25);
      const numEl = document.querySelector(it.n); const o = { v: 0 };
      tl.set(numEl, { opacity: 0 }, 0); tl.set(numEl, { opacity: 1 }, at + 0.18);
      tl.to(o, { v: it.to, duration: 1.0, ease: "power3.out", onUpdate: () => { numEl.textContent = it.pre + o.v.toFixed(it.dec) + it.post; } }, at + 0.18);
      tl.fromTo(numEl, { scale: 1.1 }, { scale: 1, duration: 0.3, ease: "expo.out", immediateRender: false }, at + 1.18);
      const path = document.querySelector(it.p); const L = path.getTotalLength();
      path.style.strokeDasharray = L; path.style.strokeDashoffset = L;
      tl.to(path, { attr: { "stroke-dashoffset": 0 }, duration: 1.0, ease: "power3.inOut" }, at + 0.22);
    });
    tl.to("#t0", { y: -10, scale: 1.04, duration: 0.55, ease: "back.out(1.8)" }, 2.0);
    tl.call(() => document.querySelector("#t0").classList.add("hero"), null, 2.0);
    tl.to("#t1", { opacity: 0.55, filter: "blur(2px)", duration: 0.45, ease: "power2.inOut" }, 2.0);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 2: Kinetic Slam Hook + Live Counter Pill (Frame-0 Ready)
Best for: `0.0–3.0s` opening hook with instant frame-0 ink, bold accent word, and living counter.

```html
<div class="clip" data-start="0" data-duration="3.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .bg{position:absolute;inset:0;background:radial-gradient(80% 55% at 50% 28%,color-mix(in srgb,var(--brand-accent,#F05A00) 28%,transparent),transparent 72%)}
    .wrap{position:absolute;left:7vw;right:7vw;top:9vh;height:50vh;display:flex;flex-direction:column;justify-content:center;gap:2.8vh}
    .pill{display:inline-flex;align-items:center;gap:1.4vmin;align-self:flex-start;padding:1.2vmin 2.4vmin;border-radius:999px;
      background:rgba(255,255,255,.08);box-shadow:inset 0 1px 0 rgba(255,255,255,.35),inset 0 0 0 1px rgba(255,255,255,.12);
      font-size:clamp(13px,2.6vmin,20px);font-weight:700;font-variant-numeric:tabular-nums}
    .dot{width:1.4vmin;height:1.4vmin;border-radius:50%;background:var(--brand-accent,#F05A00);box-shadow:0 0 14px var(--brand-accent,#F05A00)}
    .h1{font-size:clamp(36px,9.4vmin,88px);font-weight:800;letter-spacing:-0.025em;line-height:0.96}
    .lit{color:var(--brand-accent-secondary,#F7B733);text-shadow:0 0 28px color-mix(in srgb,var(--brand-accent,#F05A00) 60%,transparent)}
    .w{display:inline-block}
  </style>
  <div class="bg"></div>
  <div class="wrap" id="rig">
    <div class="pill" id="pill"><span class="dot" id="dot"></span><span>01 / 05</span><span id="tc" style="opacity:.8"></span></div>
    <div class="h1" id="h1"><span class="w lit">10</span> <span class="w">hours</span><br/><span class="w">saved</span> <span class="w">weekly.</span></div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    // Frame 0 already has strong ink; words slam cleanly by 0.35s
    tl.fromTo("#pill", { opacity: 0.85, scale: 1.05 }, { opacity: 1, scale: 1, duration: 0.4, ease: "expo.out" }, 0);
    tl.fromTo("#h1 .w", { opacity: 0, scale: 1.8, filter: "blur(12px)" }, { opacity: 1, scale: 1, filter: "blur(0px)", duration: 0.35, stagger: 0.06, ease: "expo.out" }, 0);
    const tc = document.querySelector("#tc"); const o = { v: 0 };
    tl.to(o, { v: 10, duration: 1.2, ease: "power3.out", onUpdate: () => { tc.textContent = "· " + Math.round(o.v) + " HRS"; } }, 0.15);
    [0.5, 1.0, 1.5, 2.0, 2.5].forEach((t) => {
      tl.fromTo("#dot", { scale: 1.4, opacity: 1 }, { scale: 1, opacity: 0.55, duration: 0.4, ease: "power2.out", immediateRender: false }, t);
    });
    tl.fromTo("#rig", { scale: 1.03 }, { scale: 1, duration: 0.35, ease: "expo.out", immediateRender: false }, 1.5);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 3: Transparent Floating Strike-Correct / Verdict Overlay
Best for: myth-busting or sharp verdict over `V1` talking head (pair with `timeline.set_transform(track_id="g1", y=-0.24)`).

```html
<div class="clip" data-start="0" data-duration="3.0" style="display:grid;place-items:center;width:100vw;height:100vh;background:transparent;font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .card{position:relative;padding:20px 30px;border-radius:20px;max-width:82vw;text-align:center;
      background:color-mix(in srgb,var(--brand-surface,#1A0E0E) 92%,transparent);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.3),inset 0 0 0 1px rgba(255,255,255,.12),0 24px 50px -18px rgba(0,0,0,.78);
      color:var(--brand-ink,#F7F5EF)}
    .old{position:relative;display:inline-block;font-size:clamp(20px,4.2vmin,32px);font-weight:700;opacity:.75}
    .strike{position:absolute;left:-4%;right:-4%;top:50%;height:4px;border-radius:4px;background:#FF5A5F;transform-origin:left center;transform:scaleX(0)}
    .new{margin-top:8px;font-size:clamp(24px,5.2vmin,42px);font-weight:800;letter-spacing:-0.025em;color:#2BD67B}
  </style>
  <div class="card" id="card">
    <div class="old">Post every day<span class="strike" id="st"></span></div>
    <div class="new" id="nw">Post 3 sharp hooks</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#card", { y: 24, scale: 0.92, opacity: 0 }, { y: 0, scale: 1, opacity: 1, duration: 0.35, ease: "expo.out" }, 0);
    tl.to("#st", { scaleX: 1, duration: 0.28, ease: "power3.inOut" }, 0.65);
    tl.to(".old", { opacity: 0.42, duration: 0.25, ease: "power2.out" }, 0.72);
    tl.fromTo("#nw", { y: 14, scale: 1.15, opacity: 0 }, { y: 0, scale: 1, opacity: 1, duration: 0.4, ease: "back.out(2.0)" }, 0.95);
    tl.to("#card", { y: -18, opacity: 0, filter: "blur(8px)", duration: 0.3, ease: "power3.in", immediateRender: false }, 2.68);
    window.__timelines = { main: tl };
  </script>
</div>
```
