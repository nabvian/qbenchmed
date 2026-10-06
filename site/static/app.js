// Q-BenchMed playground: published results render instantly; live runs go to
// the Pyodide worker (or, with ?backend=local, to site/dev_server.py for testing).
const $ = (selector) => document.querySelector(selector);
const pct = (value, digits = 0) => (value == null ? "—" : `${(value * 100).toFixed(digits)}%`);
const num = (value, digits = 2) => (value == null ? "—" : Number(value).toFixed(digits));
const escapeHtml = (value) => String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const SVG = "http://www.w3.org/2000/svg";

function el(name, attrs = {}, parent) {
  const node = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  if (parent) parent.appendChild(node);
  return node;
}

function table(container, columns, rows) {
  const head = columns.map((c) => `<th class="${c.num ? "num" : ""}">${escapeHtml(c.label)}</th>`).join("");
  const body = rows.map((row) => `<tr>${columns.map((c) => `<td class="${c.num ? "num" : ""}">${c.html ? c.html(row) : escapeHtml(c.value(row))}</td>`).join("")}</tr>`).join("");
  container.innerHTML = `<div class="table-scroll"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
}

// Charts draw at their container's real width (1 SVG unit = 1 CSS px) and redraw on resize.
const chartWidth = (container) => Math.max(280, Math.round(container.clientWidth || 720));
function responsive(container, draw) {
  draw();
  let last = container.clientWidth;
  new ResizeObserver(() => { if (Math.abs(container.clientWidth - last) > 4) { last = container.clientWidth; draw(); } }).observe(container);
}

// ---------- tooltip ----------
function tooltip(container) {
  const tip = document.createElement("div");
  tip.className = "tooltip";
  tip.hidden = true;
  container.appendChild(tip);
  return {
    show(html, x, y) {
      tip.innerHTML = html;
      tip.hidden = false;
      const box = container.getBoundingClientRect();
      const width = tip.offsetWidth;
      const left = Math.min(Math.max(0, x + 12), box.width - width);
      tip.style.left = `${left}px`;
      tip.style.top = `${Math.max(0, y - tip.offsetHeight - 10)}px`;
    },
    hide() { tip.hidden = true; },
  };
}
const localPoint = (container, event) => { const box = container.getBoundingClientRect(); return [event.clientX - box.left, event.clientY - box.top]; };

// ---------- chart: horizontal bars (success rate) ----------
function barChart(container, rows) {
  container.innerHTML = "";
  const rowHeight = 34, labelWidth = 104, valueWidth = 48, width = chartWidth(container), height = rows.length * rowHeight + 26;
  const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": "Share of instances reaching the certified optimum, by method" }, container);
  const plotWidth = width - labelWidth - valueWidth;
  const x = (value) => labelWidth + value * plotWidth;
  for (const tick of [0, 0.25, 0.5, 0.75, 1]) {
    el("line", { class: "gridline", x1: x(tick), x2: x(tick), y1: 0, y2: height - 22 }, svg);
    el("text", { x: x(tick), y: height - 6, "text-anchor": "middle" }, svg).textContent = `${tick * 100}%`;
  }
  const tip = tooltip(container);
  rows.forEach((row, index) => {
    const y = index * rowHeight + 6, barHeight = 20;
    el("text", { x: labelWidth - 10, y: y + 15, "text-anchor": "end", class: "label-strong" }, svg).textContent = row.method;
    const w = Math.max(2, row.success_rate * plotWidth);
    el("path", { d: roundedBar(labelWidth, y, w, barHeight, 4), fill: row.family === "quantum" ? "var(--series-2)" : "var(--series-1)" }, svg);
    el("text", { x: labelWidth + w + 8, y: y + 15 }, svg).textContent = pct(row.success_rate);
    const hit = el("rect", { x: 0, y: y - 4, width, height: rowHeight, fill: "transparent" }, svg);
    hit.addEventListener("pointermove", (event) => {
      const [px, py] = localPoint(container, event);
      tip.show(`<b>${escapeHtml(row.method)}</b><br><span class="k">reaches optimum</span> ${pct(row.success_rate)} (${Math.round(row.success_rate * row.instances)} of ${row.instances})<br><span class="k">formulation</span> ${escapeHtml(row.formulation)}`, px, py);
    });
    hit.addEventListener("pointerleave", () => tip.hide());
  });
}
// Rounded only at the data end; square at the baseline.
function roundedBar(x, y, w, h, r) {
  const radius = Math.min(r, w / 2, h / 2);
  return `M${x},${y} H${x + w - radius} Q${x + w},${y} ${x + w},${y + radius} V${y + h - radius} Q${x + w},${y + h} ${x + w - radius},${y + h} H${x} Z`;
}

// ---------- chart: coverage curve (certified vs greedy) ----------
function curveChart(container, curve, livePoints = []) {
  container.innerHTML = "";
  const width = chartWidth(container), height = width < 520 ? 240 : 300, m = { top: 14, right: width < 520 ? 70 : 92, bottom: 34, left: 44 };
  const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": "Coverage by panel size on QBMED-HEME-001: certified optimum versus greedy" }, container);
  const maxBudget = Math.max(...curve.map((d) => d.budget));
  const x = (b) => m.left + ((b - 1) / (maxBudget - 1)) * (width - m.left - m.right);
  const y = (v) => m.top + (1 - v) * (height - m.top - m.bottom);
  for (const tick of [0, 0.25, 0.5, 0.75, 1]) {
    el("line", { class: "gridline", x1: m.left, x2: width - m.right, y1: y(tick), y2: y(tick) }, svg);
    el("text", { x: m.left - 8, y: y(tick) + 4, "text-anchor": "end" }, svg).textContent = `${tick * 100}%`;
  }
  for (const tick of width < 520 ? [1, 20, 40, 66] : [1, 10, 20, 30, 40, 50, 60, 66]) el("text", { x: x(tick), y: height - 12, "text-anchor": "middle" }, svg).textContent = tick;
  el("text", { x: (m.left + width - m.right) / 2, y: height, "text-anchor": "middle" }, svg).textContent = "panel size (at most K biomarkers)";
  const line = (key) => curve.map((d, i) => `${i ? "L" : "M"}${x(d.budget).toFixed(1)},${y(d[key]).toFixed(1)}`).join("");
  el("path", { d: line("greedy"), fill: "none", stroke: "var(--series-2)", "stroke-width": 2, "stroke-linejoin": "round" }, svg);
  el("path", { d: line("certified"), fill: "none", stroke: "var(--series-1)", "stroke-width": 2, "stroke-linejoin": "round" }, svg);
  const last = curve.at(-1);
  const compact = width < 520;
  el("text", { x: width - m.right + 6, y: y(last.certified) - 4, class: "label-strong" }, svg).textContent = compact ? pct(last.certified, 2) : `certified ${pct(last.certified, 2)}`;
  el("text", { x: width - m.right + 6, y: y(last.greedy) + 14, class: "label-strong" }, svg).textContent = compact ? pct(last.greedy, 2) : `greedy ${pct(last.greedy, 2)}`;
  for (const point of livePoints) {
    for (const [key, color] of [["greedy", "var(--series-2)"], ["certified", "var(--series-1)"]]) {
      el("circle", { cx: x(point.budget), cy: y(point[key]), r: 5, fill: color, stroke: "var(--surface-1)", "stroke-width": 2 }, svg);
    }
  }
  const cross = el("line", { class: "ref", y1: m.top, y2: height - m.bottom, visibility: "hidden" }, svg);
  const tip = tooltip(container);
  const overlay = el("rect", { x: m.left, y: m.top, width: width - m.left - m.right, height: height - m.top - m.bottom, fill: "transparent" }, svg);
  overlay.addEventListener("pointermove", (event) => {
    const box = svg.getBoundingClientRect();
    const sx = ((event.clientX - box.left) / box.width) * width;
    const budget = Math.round(1 + ((sx - m.left) / (width - m.left - m.right)) * (maxBudget - 1));
    const d = curve.find((row) => row.budget === Math.max(1, Math.min(maxBudget, budget)));
    if (!d) return;
    cross.setAttribute("x1", x(d.budget)); cross.setAttribute("x2", x(d.budget)); cross.setAttribute("visibility", "visible");
    const live = livePoints.find((p) => p.budget === d.budget);
    const [px, py] = localPoint(container, event);
    tip.show(`<b>K = ${d.budget}</b><br><span class="k">certified</span> ${pct(d.certified, 2)}<br><span class="k">greedy</span> ${pct(d.greedy, 2)}${live ? `<br><span class="k">live re-solve</span> ${pct(live.certified, 2)} / ${pct(live.greedy, 2)}` : ""}`, px, py);
  });
  overlay.addEventListener("pointerleave", () => { tip.hide(); cross.setAttribute("visibility", "hidden"); });
}

// ---------- chart: lift over random (log-scale dot plot) ----------
function liftChart(container, rows) {
  container.innerHTML = "";
  const depths = [...new Set(rows.map((r) => r.depth))].sort();
  const width = chartWidth(container), rowHeight = 52, m = { top: 18, right: 24, bottom: 36, left: 44 };
  const height = m.top + depths.length * rowHeight + m.bottom;
  const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": "Median lift over a random valid panel, penalty versus constrained QAOA, log scale" }, container);
  const lo = Math.log10(0.01), hi = Math.log10(30);
  const x = (v) => m.left + ((Math.log10(v) - lo) / (hi - lo)) * (width - m.left - m.right);
  for (const tick of [0.01, 0.1, 1, 10]) {
    el("line", { class: tick === 1 ? "ref" : "gridline", x1: x(tick), x2: x(tick), y1: m.top - 6, y2: height - m.bottom }, svg);
    el("text", { x: x(tick), y: height - m.bottom + 18, "text-anchor": "middle" }, svg).textContent = `${tick}×`;
  }
  el("text", { x: x(1) + 6, y: m.top + 2, class: "label-strong" }, svg).textContent = "random valid panel";
  el("text", { x: (m.left + width - m.right) / 2, y: height - 2, "text-anchor": "middle" }, svg).textContent = "probability of the optimum ÷ random (log scale)";
  const tip = tooltip(container);
  depths.forEach((depth, index) => {
    const cy = m.top + index * rowHeight + rowHeight / 2 + 6;
    el("text", { x: m.left - 10, y: cy + 4, "text-anchor": "end", class: "label-strong" }, svg).textContent = `p=${depth}`;
    el("line", { class: "gridline", x1: m.left, x2: width - m.right, y1: cy, y2: cy }, svg);
    for (const row of rows.filter((r) => r.depth === depth)) {
      const value = Math.max(0.01, row.median_lift);
      const color = row.ansatz === "constrained" ? "var(--series-1)" : "var(--series-2)";
      el("circle", { cx: x(value), cy, r: 7, fill: color, stroke: "var(--surface-1)", "stroke-width": 2 }, svg);
      el("text", { x: x(value), y: cy - 13, "text-anchor": "middle" }, svg).textContent = `${num(row.median_lift, 2)}×`;
      const hit = el("circle", { cx: x(value), cy, r: 16, fill: "transparent" }, svg);
      hit.addEventListener("pointermove", (event) => {
        const [px, py] = localPoint(container, event);
        tip.show(`<b>${row.ansatz} QAOA, p=${depth}</b><br><span class="k">median lift</span> ${num(row.median_lift, 2)}×<br><span class="k">beats random on</span> ${row.beats_random} of ${row.instances}`, px, py);
      });
      hit.addEventListener("pointerleave", () => tip.hide());
    }
  });
}

// ---------- backends ----------
function workerBackend(onStatus) {
  const worker = new Worker("worker.js");
  const pending = new Map();
  let sequence = 0, readyResolve, readyReject;
  const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
  worker.onmessage = ({ data }) => {
    if (data.type === "status") onStatus(data.text);
    else if (data.type === "ready") readyResolve();
    else if (data.type === "failed") readyReject(new Error(data.error));
    else if (data.type === "result") {
      const job = pending.get(data.id); pending.delete(data.id);
      if (data.ok) job.resolve(data.result); else job.reject(new Error(data.error));
    }
  };
  worker.onerror = (event) => readyReject(new Error(event.message || "worker failed to start"));
  return {
    ready,
    call: (fn, ...args) => new Promise((resolve, reject) => { const id = ++sequence; pending.set(id, { resolve, reject }); worker.postMessage({ id, fn, args }); }),
  };
}

function localBackend() {
  return {
    ready: Promise.resolve(),
    call: async (fn, ...args) => {
      const response = await fetch(`/api/${fn}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args) });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error ?? `HTTP ${response.status}`);
      return payload;
    },
  };
}

// ---------- page ----------
const runtime = $("#runtime"), runtimeText = $("#runtime-text");
const setRuntime = (text, state) => { runtimeText.textContent = text; runtime.classList.toggle("ready", state === "ready"); runtime.classList.toggle("error", state === "error"); };
const useLocal = new URLSearchParams(location.search).get("backend") === "local";
const backend = useLocal ? localBackend() : workerBackend((text) => setRuntime(text));
let published = null, livePoints = [];

async function renderPublished() {
  published = await (await fetch("data/published.json")).json();
  responsive($("#chart-headtohead"), () => barChart($("#chart-headtohead"), published.headtohead));
  table($("#table-headtohead"), [
    { label: "method", value: (r) => r.method }, { label: "formulation", value: (r) => r.formulation },
    { label: "reaches optimum", num: true, value: (r) => pct(r.success_rate) }, { label: "instances", num: true, value: (r) => r.instances },
  ], published.headtohead);
  responsive($("#chart-heme"), () => curveChart($("#chart-heme"), published.flagship_curve, livePoints));
  table($("#table-heme"), [
    { label: "K", num: true, value: (r) => r.budget }, { label: "certified", num: true, value: (r) => pct(r.certified, 2) }, { label: "greedy", num: true, value: (r) => pct(r.greedy, 2) },
  ], published.flagship_curve);
  responsive($("#chart-lift"), () => liftChart($("#chart-lift"), published.constrained_mixer));
  table($("#table-lift"), [
    { label: "ansatz", value: (r) => r.ansatz }, { label: "depth", num: true, value: (r) => r.depth },
    { label: "median lift", num: true, value: (r) => `${num(r.median_lift, 2)}×` }, { label: "beats random", num: true, value: (r) => `${r.beats_random}/${r.instances}` },
  ], published.constrained_mixer);
  table($("#table-landscape"), [
    { label: "inputs", num: true, value: (r) => r.n_inputs }, { label: "qubits", num: true, value: (r) => r.qubits },
    { label: "penalty ÷ signal", num: true, value: (r) => `${num(r.penalty_dominance, 0)}×` }, { label: "useful band", num: true, value: (r) => `${num(r.useful_band_pct, 2)}%` },
  ], published.penalty_landscape);
}

// Playground controls
const form = $("#play-form");
form.n_inputs.addEventListener("input", () => { form.n_out.value = form.n_inputs.value; });
form.budget.addEventListener("input", () => { form.b_out.value = `${form.budget.value}%`; });

const resultColumns = [
  { label: "method", html: (r) => `<b>${escapeHtml(r.method)}</b>` },
  { label: "formulation", value: (r) => r.formulation ?? "" },
  { label: "QUBO energy", num: true, value: (r) => num(r.qubo_energy, 3) },
  { label: "gap to optimum", num: true, value: (r) => num(r.gap, 3) },
  { label: "found optimum", html: (r) => (r.pending ? `<span class="pill run">running…</span>` : r.error ? `<span class="pill no">error</span>` : r.found_optimum ? `<span class="pill yes">yes</span>` : `<span class="pill no">no</span>`) },
  { label: "coverage", num: true, value: (r) => pct(r.coverage, 1) },
  { label: "time", num: true, value: (r) => (r.runtime_ms == null ? "" : r.runtime_ms < 1000 ? `${Math.round(r.runtime_ms)} ms` : `${(r.runtime_ms / 1000).toFixed(1)} s`) },
];

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = $("#run-button");
  button.disabled = true;
  const depths = [...form.querySelectorAll('input[name="depth"]:checked')].map((input) => Number(input.value));
  const args = [form.regime.value, Number(form.n_inputs.value), Math.max(0, Math.floor(Number(form.seed.value) || 0)), Number(form.budget.value) / 100, depths];
  $("#play-output").hidden = false;
  $("#constrained-card").hidden = true;
  $("#instance-chips").innerHTML = `<span class="chip">Preparing instance…</span>`;
  $("#results-table").innerHTML = "";
  try {
    const info = await backend.call("prepare", ...args);
    $("#instance-chips").innerHTML = [
      ["instance", info.instance_id], ["outcomes", info.n_outcomes], ["budget K", info.budget],
      ["QUBO variables (qubits)", info.qubo_variables], ["γ period", info.gamma_period.toFixed(4)],
    ].map(([k, v]) => `<span class="chip">${escapeHtml(k)} <b>${escapeHtml(v)}</b></span>`).join("");
    const rows = info.plan.map((method) => ({ method, pending: true }));
    table($("#results-table"), resultColumns, rows);
    for (let index = 0; index < rows.length; index += 1) {
      try { rows[index] = await backend.call("run_step", index); }
      catch (error) { rows[index] = { method: rows[index].method, error: error.message }; }
      table($("#results-table"), resultColumns, rows);
    }
    if (form.constrained.checked && depths.length) {
      $("#constrained-card").hidden = false;
      $("#constrained-table").innerHTML = `<p class="muted">Running penalty and constrained QAOA…</p>`;
      const comparison = await backend.call("constrained_comparison", depths);
      table($("#constrained-table"), [
        { label: "ansatz", html: (r) => `<b>${escapeHtml(r.ansatz)}</b>` }, { label: "qubits", num: true, value: (r) => r.qubits },
        { label: "P(optimum)", num: true, value: (r) => num(r.p_optimum, 4) }, { label: "vs random", num: true, value: (r) => (r.lift == null ? "—" : `${num(r.lift, 2)}×`) },
      ], [{ ansatz: "random valid panel", depth: 0, qubits: info.n_inputs, p_optimum: comparison.random_p, lift: 1 }, ...comparison.rows.map((r) => ({ ...r, ansatz: `${r.ansatz} p=${r.depth}` }))]);
    }
  } catch (error) {
    $("#instance-chips").innerHTML = `<span class="chip">Run failed: ${escapeHtml(error.message)}</span>`;
  } finally {
    button.disabled = false;
  }
});

$("#heme-button").addEventListener("click", async () => {
  const button = $("#heme-button"), statusText = $("#heme-status");
  button.disabled = true;
  statusText.textContent = "Solving the 66-biomarker instance with the certified ILP and greedy…";
  try {
    const budgets = [5, 10, 20, 30, 33, 37, 45, 66];
    const started = performance.now();
    const live = await backend.call("heme_curve", budgets);
    livePoints = live.rows;
    curveChart($("#chart-heme"), published.flagship_curve, livePoints);
    const matches = live.rows.every((row) => {
      const ref = published.flagship_curve.find((d) => d.budget === row.budget);
      return ref && Math.abs(ref.certified - row.certified) < 1e-9 && Math.abs(ref.greedy - row.greedy) < 1e-9;
    });
    statusText.textContent = `Re-solved ${live.rows.length} budgets in ${((performance.now() - started) / 1000).toFixed(1)} s · ceiling ${pct(live.ceiling, 2)}, ${live.unreachable} outcomes unreachable · ${matches ? "matches the published table ✓" : "differs from the published table"}`;
  } catch (error) {
    statusText.textContent = `Live solve failed: ${error.message}`;
  } finally {
    button.disabled = false;
  }
});

renderPublished().catch((error) => console.error("published data", error));
backend.ready.then(
  () => { setRuntime(useLocal ? "Ready · local test backend" : "Ready · Python running in your browser", "ready"); $("#run-button").disabled = false; $("#heme-button").disabled = false; },
  (error) => setRuntime(`Python runtime failed to load (${error.message}). Published results are still shown.`, "error"),
);
