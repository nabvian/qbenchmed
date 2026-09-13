"use strict";

const state = { bootstrap: null, workflow: null, sourceType: "archive", runMode: "express", busy: false, reportLists: Object.create(null) };
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => Array.from(document.querySelectorAll(selector));

document.addEventListener("DOMContentLoaded", boot);

async function boot() {
  wireEvents();
  try {
    state.bootstrap = await api("/api/bootstrap");
    $("#service-status").classList.add("connected");
    $("#service-status").innerHTML = '<span aria-hidden="true"></span> Local service connected';
    $("#reviewer-name").value = localStorage.getItem("qbm-reviewer") || "local-reviewer";
    renderRecent(state.bootstrap.recent_runs);
    const match = location.hash.match(/^#run\/([0-9a-f-]+)$/i);
    if (match) await loadRun(match[1]);
  } catch (error) {
    showHomeError(error.message || "The local Q-BenchMed service is unavailable.");
    $("#service-status").textContent = "Service disconnected";
  }
}

function wireEvents() {
  $$('input[name="source-type"]').forEach((input) => input.addEventListener("change", () => selectSource(input.value)));
  $$('input[name="run-mode"]').forEach((input) => input.addEventListener("change", () => selectRunMode(input.value)));
  $("#archive-file").addEventListener("change", onArchiveSelected);
  $("#folder-files").addEventListener("change", onFolderSelected);
  $("#github-url").addEventListener("input", autofillGithubName);
  $("#start-audit").addEventListener("click", startAudit);
  $("#new-audit").addEventListener("click", showHome);
  $("#copy-run-id").addEventListener("click", () => copyText(state.workflow?.run_id, "Run ID copied"));
  const drop = $("#archive-drop");
  ["dragenter", "dragover"].forEach((name) => drop.addEventListener(name, (event) => { event.preventDefault(); drop.classList.add("dragging"); }));
  ["dragleave", "drop"].forEach((name) => drop.addEventListener(name, (event) => { event.preventDefault(); drop.classList.remove("dragging"); }));
  drop.addEventListener("drop", (event) => {
    const file = event.dataTransfer.files[0];
    if (!file) return;
    const transfer = new DataTransfer(); transfer.items.add(file); $("#archive-file").files = transfer.files; onArchiveSelected();
  });
  window.addEventListener("hashchange", () => {
    const match = location.hash.match(/^#run\/([0-9a-f-]+)$/i);
    if (match && state.workflow?.run_id !== match[1]) loadRun(match[1]);
  });
}

function selectRunMode(mode) {
  state.runMode = mode;
  $$(".mode-choice").forEach((label) => label.classList.toggle("selected", label.querySelector("input").value === mode));
}

function selectSource(type) {
  state.sourceType = type;
  $$(".source-choice").forEach((label) => label.classList.toggle("selected", label.querySelector("input").value === type));
  $$(".source-panel").forEach((panel) => panel.classList.toggle("hidden", panel.dataset.panel !== type));
  hideHomeError();
}

function onArchiveSelected() {
  const file = $("#archive-file").files[0];
  if (!file) return;
  const target = $("#archive-selected");
  target.classList.remove("hidden");
  target.innerHTML = `<span><strong>${escapeHtml(file.name)}</strong><small>${formatBytes(file.size)}</small></span><span>Ready</span>`;
  if (!$("#project-name").value) $("#project-name").value = file.name.replace(/\.(tar\.gz|tgz|zip|tar)$/i, "");
}

function onFolderSelected() {
  const files = Array.from($("#folder-files").files);
  if (!files.length) return;
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const root = (files[0].webkitRelativePath || files[0].name).split("/")[0];
  const target = $("#folder-selected");
  target.classList.remove("hidden");
  target.innerHTML = `<span><strong>${escapeHtml(root)}</strong><small>${files.length.toLocaleString()} files · ${formatBytes(total)}</small></span><span>Ready</span>`;
  if (!$("#project-name").value) $("#project-name").value = root;
}

function autofillGithubName() {
  if ($("#project-name").value) return;
  try {
    const url = new URL($("#github-url").value);
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts.length === 2) $("#project-name").value = parts[1].replace(/\.git$/i, "");
  } catch (_) { /* incomplete URL */ }
}

async function startAudit() {
  if (state.busy) return;
  hideHomeError();
  const reviewer = $("#reviewer-name").value.trim();
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(reviewer)) {
    showHomeError("Reviewer label must use 1–64 letters, digits, dots, underscores, or hyphens."); return;
  }
  localStorage.setItem("qbm-reviewer", reviewer);
  const projectName = $("#project-name").value.trim();
  state.busy = true;
  setView("processing");
  const acquiring = state.sourceType === "github" ? "Resolving the repository to one immutable commit…" : "Uploading and validating the selected source…";
  $("#processing-phase").textContent = state.runMode === "express" ? `${acquiring} Then every stage runs without stopping, which can take a minute on a large profile.` : acquiring;
  try {
    let workflow;
    if (state.sourceType === "archive") {
      const file = $("#archive-file").files[0];
      if (!file) throw new Error("Choose an archive before starting the audit.");
      const form = new FormData(); if (projectName) form.append("project_name", projectName); form.append("run_mode", state.runMode); form.append("source", file, file.name);
      workflow = await api("/api/import/archive", { method: "POST", body: form });
    } else if (state.sourceType === "folder") {
      const files = Array.from($("#folder-files").files);
      if (!files.length) throw new Error("Choose a folder before starting the audit.");
      const form = new FormData(); if (projectName) form.append("project_name", projectName); form.append("run_mode", state.runMode);
      form.append("manifest", JSON.stringify({ paths: files.map((file) => file.webkitRelativePath || file.name) }));
      files.forEach((file) => form.append("file", file, file.name));
      workflow = await api("/api/import/folder", { method: "POST", body: form });
    } else {
      const repositoryUrl = $("#github-url").value.trim();
      if (!repositoryUrl) throw new Error("Enter a public GitHub repository URL.");
      workflow = await api("/api/import/github", {
        method: "POST", json: { repository_url: repositoryUrl, reference: $("#github-ref").value.trim() || null, project_name: projectName || null, run_mode: state.runMode }
      });
    }
    state.workflow = workflow;
    location.hash = `run/${workflow.run_id}`;
    renderWorkflow(workflow);
  } catch (error) {
    setView("home"); showHomeError(error.message || "The audit could not be started.");
  } finally { state.busy = false; }
}

async function loadRun(runId) {
  setView("processing"); $("#processing-phase").textContent = "Restoring the exact persisted audit state…";
  try { const workflow = await api(`/api/runs/${encodeURIComponent(runId)}`); state.workflow = workflow; renderWorkflow(workflow); }
  catch (error) { setView("home"); showHomeError(error.message || "That audit could not be restored."); }
}

function renderWorkflow(workflow) {
  state.workflow = workflow; state.reportLists = Object.create(null); setView("workspace"); $("#new-audit").classList.remove("hidden");
  $("#workspace-title").textContent = workflow.project.display_name;
  $("#run-source-kind").textContent = sourceKindLabel(workflow.source.kind);
  $("#run-meta").textContent = `${workflow.source.source_locator} · ${shortHash(workflow.source.content_sha256)}`;
  $("#run-status").textContent = statusLabel(workflow.run_state);
  $("#timeline-mode").textContent = workflow.run_mode === "express" ? "Express workflow" : "Governed workflow";
  $("#integrity-card").innerHTML = `<span aria-hidden="true">${workflow.event_chain_valid ? "✓" : "!"}</span><div><strong>Event chain</strong><small>${workflow.event_chain_valid ? "Verified and continuous" : "Verification failed"}</small></div>`;
  $("#stage-list").innerHTML = workflow.stages.map((stage, index) => {
    const status = stage.status === "approved" && workflow.run_state === "complete" && index === workflow.stages.length - 1 ? "complete" : stage.status;
    const marker = status === "approved" || status === "complete" ? "✓" : status === "not_applicable" || status === "skipped" ? "—" : index + 1;
    return `<li class="stage-item ${escapeHtml(status)}"><span class="stage-dot">${marker}</span><div class="stage-copy"><strong>${escapeHtml(stage.label)}</strong><small>${statusLabel(status)}</small>${stage.output ? `<code>${shortHash(stage.output.output_hash)}</code>` : ""}</div></li>`;
  }).join("");
  if (workflow.run_state === "complete" && workflow.full_report) renderComplete(workflow); else if (workflow.run_state === "complete") renderSourceOnlyComplete(workflow); else renderCurrentStage(workflow);
}

function renderCurrentStage(workflow) {
  const stage = workflow.stages.find((item) => item.status === "waiting_approval") || workflow.stages.find((item) => ["needs_changes", "rejected"].includes(item.status)) || [...workflow.stages].reverse().find((item) => item.output && item.status === "approved") || workflow.stages[0];
  const summary = stageSummary(stage.id, workflow);
  const canApprove = stage.id !== "inventory" || !workflow.inventory?.blocked_entries;
  const terminal = ["rejected", "needs_changes"].includes(workflow.run_state);
  const waiting = stage.status === "waiting_approval";
  $("#inspector").innerHTML = `
    <article class="inspector-card">
      <header class="inspector-header"><div class="stage-kicker"><span></span> ${waiting ? "Awaiting your review" : statusLabel(stage.status)}</div><h2 tabindex="-1" id="active-stage-title">${escapeHtml(stage.label)}</h2><p>${escapeHtml(stage.purpose)}</p>${stageScopeBadge(stage.id)}</header>
      <div class="inspector-body">${summary}<div class="section-block"><h3>Exact result identity</h3><div class="hash-box"><code>${escapeHtml(stage.output?.output_hash || "Not produced")}</code>${stage.output ? '<button class="copy-button" type="button" data-copy-hash>Copy hash</button>' : ""}</div></div><details class="technical"><summary>Technical details and raw JSON</summary><pre>${escapeHtml(JSON.stringify(stagePayload(stage.id, workflow), null, 2))}</pre></details></div>
      ${waiting ? approvalFooter(canApprove, stage) : terminal ? `<div class="approval-footer"><p class="approval-note">This immutable run is ${statusLabel(workflow.run_state)}. Start a new audit with corrected source material to continue.</p><button class="primary-button" type="button" data-new-audit>Audit corrected source</button></div>` : `<div class="approval-footer"><p class="approval-note">The exact result is approved. Continue safely if the next producer did not start automatically.</p><button class="primary-button" type="button" data-advance>Continue audit</button></div>`}
    </article>`;
  bindInspector(stage);
  bindReportDrilldowns();
  requestAnimationFrame(() => $("#active-stage-title")?.focus());
}

function approvalFooter(canApprove, stage) {
  const blocked = !canApprove ? `<div class="inline-alert">This inventory has blocked entries. Correct the source and start a new run; it cannot be frozen safely.</div>` : "";
  return `<div class="approval-footer">${blocked}<p class="approval-note"><span aria-hidden="true">ⓘ</span> Approval accepts this exact result for the next step. It does not certify that the project is correct or issue-free.</p><label class="sr-only" for="decision-reason">Decision reason</label><textarea id="decision-reason" class="reason-input" placeholder="Reason (optional for approval; required for changes or rejection)"></textarea><div class="action-row"><button class="danger-button" type="button" data-decision="reject">Reject run</button><button class="secondary-button" type="button" data-decision="request_changes">Request changes</button><button class="primary-button" type="button" data-decision="approve" ${canApprove ? "" : "disabled"}>Approve &amp; continue →</button></div></div>`;
}

function bindInspector(stage) {
  $("[data-copy-hash]")?.addEventListener("click", () => copyText(stage.output.output_hash, "Result hash copied"));
  $("[data-new-audit]")?.addEventListener("click", showHome);
  $("[data-advance]")?.addEventListener("click", advance);
  $$('[data-decision]').forEach((button) => button.addEventListener("click", () => decide(stage, button.dataset.decision)));
}

async function decide(stage, decision) {
  if (state.busy) return;
  const reason = $("#decision-reason")?.value.trim() || null;
  if (decision !== "approve" && !reason) { showToast("Add a reason before choosing this decision."); $("#decision-reason")?.focus(); return; }
  if (decision === "reject" && !window.confirm("Reject this immutable audit run? The evidence and audit trail will be preserved.")) return;
  state.busy = true; disableActions(true);
  try {
    const workflow = await api(`/api/runs/${state.workflow.run_id}/decision`, { method: "POST", json: { stage_id: stage.id, expected_output_hash: stage.output.output_hash, decision, actor_id: $("#reviewer-name").value || "local-reviewer", reason } });
    renderWorkflow(workflow);
  } catch (error) { showInspectorError(error.message); if (error.code === "stale_output") await loadRun(state.workflow.run_id); }
  finally { state.busy = false; disableActions(false); }
}

async function advance() {
  if (state.busy) return; state.busy = true; disableActions(true);
  try { const workflow = await api(`/api/runs/${state.workflow.run_id}/advance`, { method: "POST" }); renderWorkflow(workflow); }
  catch (error) { showInspectorError(error.message); }
  finally { state.busy = false; disableActions(false); }
}

function renderComplete(workflow) {
  const sourceReport = workflow.report; const summary = sourceReport.summary; const full = workflow.full_report;
  const profileResult = workflow.biomedical_profile; const candidate = capabilityValue(profileResult); const profile = candidate?.profile;
  const headline = profile ? `${profile.inputs.length} inputs optimized against ${profile.outcomes.length} outcomes` : "Full report records why optimization was skipped";
  const exports = [`<a class="download-button" href="${escapeHtml(workflow.report_download_url)}" download>↓ Full report JSON</a>`];
  if (workflow.profile_download_url) exports.push(`<a class="secondary-download" href="${escapeHtml(workflow.profile_download_url)}" download>↓ Reusable qbm.profile</a>`);
  if (workflow.optimization_download_url) exports.push(`<a class="secondary-download" href="${escapeHtml(workflow.optimization_download_url)}" download>↓ Panel optimization JSON</a>`);
  if (workflow.wiring_diagnostic_download_url) exports.push(`<a class="secondary-download" href="${escapeHtml(workflow.wiring_diagnostic_download_url)}" download>↓ Wiring diagnostics JSON</a>`);
  if (workflow.quantum_export_url) exports.push(`<a class="secondary-download" href="${escapeHtml(workflow.quantum_export_url)}" download>↓ QUBO / Ising formulation</a>`);
  $("#inspector").innerHTML = `<article class="inspector-card"><header class="completed-hero"><span class="complete-mark" aria-hidden="true">✓</span><p class="eyebrow">Full Q-BenchMed report complete</p><h2 tabindex="-1" id="active-stage-title">${escapeHtml(headline)}</h2><p>Every material result was accepted against its exact content hash. The report distinguishes completed, unavailable and optional checks and preserves the settings needed to reproduce each result.</p>${reviewProvenance(workflow)}<div class="report-actions">${exports.join("")}<button class="quiet-button" type="button" data-new-audit>Analyze another project</button></div></header><div class="inspector-body">${profile ? profileSummary(profileResult, workflow) : `<div class="metric-grid">${metric(summary.critical, "Critical", "critical")}${metric(summary.errors, "Errors", "error")}${metric(summary.warnings, "Warnings", "warning")}${metric(summary.info, "Info", "info")}</div>`}${stageSummary("profile-comparison", workflow)}${stageSummary("classical-optimization", workflow)}${stageSummary("qubo-ising-validation", workflow)}<section class="section-block"><h3>Source audit findings (showing ${Math.min(20, sourceReport.findings.length)} of ${summary.total})</h3>${sourceReport.findings.length > 20 ? '<p>The browser shows the first 20 findings; download the full report JSON for the complete source-finding ledger.</p>' : ""}<div class="finding-list">${sourceReport.findings.length ? sourceReport.findings.slice(0, 20).map(findingCard).join("") : '<div class="empty-state">No findings were produced by the source checks that ran. This is not a claim that the project is error-free.</div>'}</div></section><details class="technical"><summary>Immutable full-report artifact</summary><pre>${escapeHtml(JSON.stringify(full, null, 2))}</pre></details></div></article>`;
  $("[data-new-audit]")?.addEventListener("click", showHome); $("[data-copy-source]")?.addEventListener("click", () => copyText(workflow.source.content_sha256, "Source hash copied"));
  bindReportDrilldowns();
  requestAnimationFrame(() => $("#active-stage-title")?.focus());
}

function renderSourceOnlyComplete(workflow) {
  const report = workflow.report; const summary = report.summary; const coverage = workflow.audit_coverage; const parser = workflow.graph?.parser_coverage;
  const headline = summary.critical ? "Critical findings need action" : summary.errors ? "Errors found in evaluated structure" : summary.warnings ? "Review recommended" : summary.info ? `${summary.info} informational observation${summary.info === 1 ? "" : "s"}` : "No findings from completed source checks";
  $("#inspector").innerHTML = `<article class="inspector-card"><header class="completed-hero source-only"><span class="complete-mark" aria-hidden="true">✓</span><p class="eyebrow">Generic source audit complete.</p><h2 tabindex="-1" id="active-stage-title">${escapeHtml(headline)}</h2><p>This legacy/source-only run ended after structural checks. It did not build an approved biomedical profile, compare versions, optimize a panel, or validate QUBO/Ising. Start a new analysis to use the full workflow.</p>${reviewProvenance(workflow)}<div class="report-actions"><a class="download-button" href="${escapeHtml(workflow.report_download_url)}" download>↓ Download source-audit JSON</a><button class="quiet-button" type="button" data-new-audit>Start full analysis</button></div></header><div class="inspector-body"><div class="metric-grid">${metric(summary.critical, "Critical", "critical")}${metric(summary.errors, "Errors", "error")}${metric(summary.warnings, "Warnings", "warning")}${metric(summary.info, "Info", "info")}</div>${parser ? parserCoverage(parser) : ""}${coverage ? checkLedger(coverage) : ""}<section class="section-block"><h3>Findings (${summary.total})</h3><div class="finding-list">${report.findings.length ? report.findings.map(findingCard).join("") : '<div class="empty-state">No findings were produced by the checks that ran. This does not prove the project is correct.</div>'}</div></section></div></article>`;
  $("[data-new-audit]")?.addEventListener("click", showHome);
  requestAnimationFrame(() => $("#active-stage-title")?.focus());
}

function reviewProvenance(workflow) {
  if (workflow.run_mode !== "express") {
    return '<div class="notice-strip"><strong>Reviewed at every checkpoint.</strong><span>A person approved each stage output against its exact content hash.</span></div>';
  }
  return '<div class="notice-strip express-notice"><strong>Nobody reviewed this run.</strong><span>It was started in express mode, so every stage was accepted by policy and the approvals are stamped <code>auto_accepted_by_policy</code> rather than with a reviewer. The stages, outputs and hashes are the same as a governed run; the sign-off is not. Re-run in governed mode when the result has to carry a human decision.</span></div>';
}

function stageSummary(id, workflow) {
  if (id === "inventory" && workflow.inventory) { const x = workflow.inventory; return `<div class="metric-grid">${metric(x.included_files, "Included files")}${metric(x.excluded_entries, "Excluded")}${metric(x.blocked_entries, "Blocked", x.blocked_entries ? "critical" : "")}${metric(formatBytes(x.total_included_bytes), "Frozen bytes")}</div>${sourceProvenance(workflow)}${x.exceptions.length ? `<section class="section-block"><h3>Exceptions</h3><div class="exception-list">${x.exceptions.slice(0, 30).map((entry) => `<div class="exception-row"><code>${escapeHtml(entry.relative_path)}</code><span>${escapeHtml(entry.reason || entry.disposition)}</span></div>`).join("")}</div></section>` : '<div class="section-block empty-state">Every observed entry was included by the active safety policy.</div>'}`; }
  if (id === "source-snapshot" && workflow.snapshot) { const x = workflow.snapshot; return `<div class="metric-grid">${metric(x.files, "Frozen files")}${metric(formatBytes(x.total_bytes), "Evidence bytes")}</div><section class="section-block"><h3>What this means</h3><p>Every later graph node and finding points back to these immutable content hashes, even if the uploaded source changes later.</p></section>`; }
  if (id === "raw-graph" && workflow.graph) { const x = workflow.graph; return `<div class="metric-grid">${metric(x.nodes, "Graph nodes")}${metric(x.edges, "Graph edges")}${metric(x.diagnostics.length, "Diagnostics")}${metric(x.language_scan?.dependency_count || 0, "Observed dependencies")}</div>${parserCoverage(x.parser_coverage)}${languageScan(x.language_scan)}`; }
  if (id === "generic-audit" && workflow.report) { const x = workflow.report.summary; return `<div class="notice-strip"><strong>Generic source audit complete.</strong><span>This is the source-evidence checkpoint, not the final Q-BenchMed result.</span></div><div class="metric-grid">${metric(x.critical, "Critical", "critical")}${metric(x.errors, "Errors", "error")}${metric(x.warnings, "Warnings", "warning")}${metric(x.info, "Info", "info")}</div>${workflow.audit_coverage ? checkLedger(workflow.audit_coverage) : ""}<section class="section-block"><h3>Top findings</h3><div class="finding-list">${workflow.report.findings.slice(0, 8).map(findingCard).join("") || '<div class="empty-state">No findings were produced by the generic checks that ran. This is not a correctness certificate.</div>'}</div></section>`; }
  if (id === "adapter-detection" && workflow.adapter_detection) { const x = workflow.adapter_detection; const eligible = (x.candidates || []).filter((item) => item.eligible); return `<div class="metric-grid">${metric((x.candidates || []).length, "Installed candidates")}${metric(eligible.length, "Eligible adapters")}${metric(eligible[0] ? formatPercent(eligible[0].confidence_bps) : "—", "Best confidence")}</div>${eligible.length ? `<section class="section-block"><h3>Eligible immutable packages</h3><div class="data-list">${eligible.map((item) => dataRow(`${item.adapter_id}@${item.adapter_version}`, formatPercent(item.confidence_bps), shortHash(item.package_hash))).join("")}</div></section>` : '<div class="empty-state">No installed data-only adapter matched. Adapter plan, semantic graph, semantic audit and projection-catalog stages will be marked not applicable; the conservative structured biomedical profiler can still produce an approval-gated candidate.</div>'}`; }
  if (id === "adapter-plan" && workflow.adapter_plan) { const x = workflow.adapter_plan; return `<div class="metric-grid">${metric((x.adapters || []).length, "Locked adapters")}${metric(shortHash(x.configuration_hash), "Configuration")}</div><div class="data-list">${(x.adapters || []).map((item) => dataRow(`${item.adapter_id}@${item.adapter_version}`, shortHash(item.package_hash), "Conformance locked")).join("")}</div>`; }
  if (id === "semantic-graph" && workflow.semantic_graph) { const x = workflow.semantic_graph; return `<div class="metric-grid">${metric(x.nodes, "Semantic nodes")}${metric(x.edges, "Semantic relationships")}${metric(x.adapters, "Adapters")}</div>`; }
  if (id === "semantic-audit" && workflow.semantic_report) { const x = workflow.semantic_report.summary || {}; return `<div class="metric-grid">${metric(x.critical || 0, "Critical", "critical")}${metric(x.errors || 0, "Errors", "error")}${metric(x.warnings || 0, "Warnings", "warning")}${metric(x.info || 0, "Info", "info")}</div>`; }
  if (id === "projection-catalog" && workflow.projection_catalog) { const x = workflow.projection_catalog; return `<div class="metric-grid">${metric(x.projections, "Declared views")}${metric(x.available, "Available")}${metric(x.unavailable, "Unavailable")}</div>`; }
  if (id === "biomedical-profile" && workflow.biomedical_profile) return profileSummary(workflow.biomedical_profile, workflow);
  if (id === "profile-comparison" && workflow.profile_comparison) return comparisonSummary(workflow.profile_comparison);
  if (id === "classical-optimization" && workflow.classical_optimization) return optimizationSummary(workflow.classical_optimization, workflow);
  if (id === "qubo-ising-validation" && workflow.qubo_ising_validation) return quboSummary(workflow.qubo_ising_validation);
  if (id === "full-report" && workflow.full_report) { const x = workflow.full_report; return `<div class="metric-grid">${metric((x.sections || []).filter((item) => item.status === "complete").length, "Complete sections")}${metric((x.sections || []).filter((item) => item.status === "skipped").length, "Skipped sections")}${metric((x.limitations || []).length, "Limitations")}${metric((x.reproducibility?.stage_outputs || []).length, "Bound outputs")}</div><section class="section-block"><h3>Report sections</h3><div class="data-list">${(x.sections || []).map((item) => dataRow(item.name, statusLabel(item.status), item.reason || "Included")).join("")}</div></section>`; }
  return '<div class="empty-state">This stage has not produced a reviewable result yet.</div>';
}

function stagePayload(id, workflow) { return ({ inventory: workflow.inventory, "source-snapshot": workflow.snapshot, "raw-graph": workflow.graph, "generic-audit": workflow.report, "adapter-detection": workflow.adapter_detection, "adapter-plan": workflow.adapter_plan, "semantic-graph": workflow.semantic_graph, "semantic-audit": workflow.semantic_report, "projection-catalog": workflow.projection_catalog, "biomedical-profile": workflow.biomedical_profile, "profile-comparison": workflow.profile_comparison, "classical-optimization": workflow.classical_optimization, "qubo-ising-validation": workflow.qubo_ising_validation, "full-report": workflow.full_report })[id]; }
function sourceProvenance(workflow) { return `<section class="section-block"><h3>Exact source provenance</h3><p>${escapeHtml(workflow.source.source_locator)}${workflow.source.requested_revision ? ` · requested <code>${escapeHtml(workflow.source.requested_revision)}</code>` : ""}${workflow.source.resolved_revision ? ` · resolved commit <code>${escapeHtml(workflow.source.resolved_revision)}</code>` : ""}</p><div class="hash-box"><code>${escapeHtml(workflow.source.content_sha256)}</code></div></section>`; }
function metric(value, label, tone = "") { return `<div class="metric ${tone}"><strong>${escapeHtml(String(value))}</strong><span>${escapeHtml(label)}</span></div>`; }
function findingCard(finding) { return `<article class="finding-card"><div class="finding-top"><span class="severity-chip ${escapeHtml(finding.severity)}">${escapeHtml(finding.severity)}</span><span class="finding-code">${escapeHtml(finding.code)}</span></div><h3>${escapeHtml(finding.title)}</h3><p>${escapeHtml(finding.message)}</p><p class="remediation"><strong>Next:</strong> ${escapeHtml(finding.remediation)}</p></article>`; }

function stageScopeBadge(id) {
  const labels = { "generic-audit": "Domain-blind source evidence", "adapter-detection": "Data-only adapters", "biomedical-profile": "Biomedical semantics · approval required", "classical-optimization": "Deterministic baselines", "qubo-ising-validation": "Quantum-ready formulation · no advantage claim" };
  return labels[id] ? `<span class="scope-badge">${escapeHtml(labels[id])}</span>` : "";
}

function capabilityValue(result) { return result?.status === "complete" ? result.value : null; }
function dataRow(name, value, note = "") { return `<div class="data-row"><strong>${escapeHtml(name)}</strong><span>${escapeHtml(value)}</span><small>${escapeHtml(note)}</small></div>`; }

function profileSummary(result, workflow = {}) {
  const candidate = capabilityValue(result);
  if (!candidate) return skippedSummary(result, "No valid qbm.profile candidate was available");
  const profile = candidate.profile;
  const index = buildProfileIndex(profile);
  const activeCount = profile.inputs.length - index.inertInputs.size;
  const reachableCount = profile.outcomes.length - index.unreachableOutcomes.size;
  const method = String(candidate.method || "unknown").replaceAll("_", " ");
  return `<div class="metric-grid">${metric(profile.inputs.length, "Inputs")}${metric(profile.outcomes.length, "Outcomes")}${metric(profile.relationships.length, "Relationships")}${metric(formatPercent(candidate.confidence_bps), "Projection confidence")}</div>
    <nav class="report-jump-nav" aria-label="Profile report sections">
      <button type="button" data-scroll-target="profile-input-inventory">All inputs <span>${profile.inputs.length}</span></button>
      <button type="button" data-scroll-target="profile-outcome-inventory">All outcomes <span>${profile.outcomes.length}</span></button>
      <button type="button" data-scroll-target="profile-relationship-inventory">Exact relationships <span>${profile.relationships.length}</span></button>
      ${capabilityValue(workflow.classical_optimization) ? '<button type="button" data-scroll-target="coverage-panel-results">K panels</button>' : ""}
    </nav>
    ${reasoningBoundary(candidate, workflow, activeCount, reachableCount, index)}
    <section class="section-block"><h3>Projection review</h3><div class="data-list">${dataRow("Method", method, "Human approval is required")}${dataRow("Extracted fields", candidate.extracted_fields.length, `${candidate.evidence.length} immutable evidence links`)}${dataRow("Defaulted fields", candidate.defaulted_fields.length, "Introduced values require review")}${dataRow("Diagnostics", candidate.diagnostics.length, `${candidate.assumptions.length} recorded assumptions`)}</div></section>
    ${profileInventory(candidate, index)}
    ${projectionReviewLedger(candidate)}`;
}

function buildProfileIndex(profile) {
  const inputRows = new Map(profile.inputs.map((input, position) => [input.id, `profile-input-${position}`]));
  const outcomeRows = new Map(profile.outcomes.map((outcome, position) => [outcome.id, `profile-outcome-${position}`]));
  const outcomesByInput = new Map(profile.inputs.map((input) => [input.id, []]));
  const inputsByOutcome = new Map(profile.outcomes.map((outcome) => [outcome.id, []]));
  profile.relationships.forEach((relationship) => {
    outcomesByInput.get(relationship.input_id)?.push(relationship.outcome_id);
    inputsByOutcome.get(relationship.outcome_id)?.push(relationship.input_id);
  });
  outcomesByInput.forEach((items) => items.sort());
  inputsByOutcome.forEach((items) => items.sort());
  return {
    inputRows,
    outcomeRows,
    outcomesByInput,
    inputsByOutcome,
    inertInputs: new Set([...outcomesByInput].filter(([, outcomes]) => outcomes.length === 0).map(([id]) => id)),
    unreachableOutcomes: new Set([...inputsByOutcome].filter(([, inputs]) => inputs.length === 0).map(([id]) => id)),
  };
}

function reasoningBoundary(candidate, workflow, activeCount, reachableCount, index) {
  const semantic = semanticLayerState(workflow);
  const profile = candidate.profile;
  const semanticTone = semantic.status === "complete" ? "wired" : semantic.status === "running" ? "partial" : "unwired";
  return `<section class="reasoning-boundary" aria-labelledby="reasoning-boundary-title">
      <div class="section-heading"><div><p class="eyebrow">Interpretation boundary</p><h3 id="reasoning-boundary-title">What “wired” and “unwired” mean here</h3></div><span class="wiring-chip ${semanticTone}">${escapeHtml(statusLabel(semantic.status))}</span></div>
      <p><strong>This is the approved <code>qbm.profile</code>, not exhaustive source truth.</strong> “Inert input” means no approved input→outcome relationship references that input. “Unreachable outcome” means no approved relationship reaches that outcome. Neither statement proves that the original project contains no logic.</p>
      <div class="boundary-grid">
        <div><strong>${activeCount} wired · ${index.inertInputs.size} inert</strong><span>Selectable inputs in this approved profile</span></div>
        <div><strong>${reachableCount} reachable · ${index.unreachableOutcomes.size} unreachable</strong><span>Outcomes in this approved profile</span></div>
        <div><strong>${escapeHtml(statusLabel(semantic.status))}</strong><span>Adapter-specific semantic reasoning layer</span></div>
      </div>
      <div class="reason-ledger"><strong>Semantic layer reason</strong><p>${escapeHtml(semantic.reason)}</p></div>
      <p class="boundary-note">Static parsing, unsupported formats, skipped adapter stages, or projection assumptions may leave real source logic outside this incidence graph. Use each row’s evidence and assumption details before changing biomedical logic.</p>
    </section>`;
}

function semanticLayerState(workflow) {
  const reported = workflow?.full_report?.semantic;
  if (reported?.status) {
    return {
      status: reported.status,
      reason: reported.reason || "The adapter-specific semantic graph, policies and projection catalog completed.",
    };
  }
  const semanticIds = new Set(["adapter-plan", "semantic-graph", "semantic-audit", "projection-catalog"]);
  const stages = (workflow?.stages || []).filter((stage) => semanticIds.has(stage.id));
  if (stages.some((stage) => ["running", "waiting_approval"].includes(stage.status))) {
    const active = stages.find((stage) => ["running", "waiting_approval"].includes(stage.status));
    return { status: "running", reason: `${active?.label || "Semantic processing"} is ${statusLabel(active?.status).toLowerCase()}.` };
  }
  if (stages.length && stages.every((stage) => ["approved", "complete"].includes(stage.status))) {
    return { status: "complete", reason: "The installed adapter-specific semantic graph, policies and projection catalog completed." };
  }
  if (stages.some((stage) => ["not_applicable", "skipped"].includes(stage.status))) {
    return { status: "skipped", reason: "No eligible installed declarative adapter was selected; adapter-specific semantic graph, policies and projection catalog checks did not run." };
  }
  return { status: "not_started", reason: "The adapter-specific semantic reasoning stages have not run yet." };
}

function profileInventory(candidate, index) {
  const profile = candidate.profile;
  const explicitProfile = candidate.method === "explicit_profile";
  const supportIndex = buildProjectionSupportIndex(candidate);
  const inputRows = profile.inputs.map((input, position) => {
    const outcomes = index.outcomesByInput.get(input.id) || [];
    const inert = outcomes.length === 0;
    const supportTarget = explicitProfile ? `/inputs/${position}/` : `/inputs/${escapePointerSegment(input.id)}/`;
    const support = projectionSupport(supportIndex, supportTarget);
    const search = [input.id, input.label, ...(input.tags || []), ...outcomes, inert ? "inert unwired" : "wired active"].join(" ").toLowerCase();
    return { id: input.id, status: inert ? "inert" : "wired", search, render: () => `<article class="entity-row" id="${index.inputRows.get(input.id)}" tabindex="-1">
        <div class="entity-heading"><span class="wiring-chip ${inert ? "unwired" : "wired"}">${inert ? "Inert" : "Wired"}</span><code>${escapeHtml(input.id)}</code><span class="entity-degree">${outcomes.length} outcome${outcomes.length === 1 ? "" : "s"}</span></div>
        <h4>${escapeHtml(input.label)}</h4>
        <div class="entity-facts"><span><strong>Cost</strong> ${escapeHtml(formatNumber(input.cost))}</span><span><strong>Tags</strong> ${tagList(input.tags)}</span></div>
        <div class="entity-connections"><strong>Covered outcomes</strong>${outcomes.length ? entityLinks(outcomes, index.outcomeRows, "outcome") : '<span class="no-connections">None in the approved profile</span>'}</div>
        ${inert ? '<p class="unwired-reason"><strong>Why marked inert:</strong> no approved relationship references this input, so the optimizer cannot use it to cover an outcome. This may be a genuine gap or a projection/parser/adapter limitation.</p>' : ""}
        ${projectionSupportDetails(support, "input")}
      </article>` };
  });
  const outcomeRows = profile.outcomes.map((outcome, position) => {
    const inputs = index.inputsByOutcome.get(outcome.id) || [];
    const unreachable = inputs.length === 0;
    const supportTarget = explicitProfile ? `/outcomes/${position}/` : `/outcomes/${escapePointerSegment(outcome.id)}/`;
    const support = projectionSupport(supportIndex, supportTarget);
    const search = [outcome.id, outcome.label, ...(outcome.tags || []), ...inputs, unreachable ? "unreachable unwired" : "reachable wired"].join(" ").toLowerCase();
    return { id: outcome.id, status: unreachable ? "unreachable" : "reachable", search, render: () => `<article class="entity-row" id="${index.outcomeRows.get(outcome.id)}" tabindex="-1">
        <div class="entity-heading"><span class="wiring-chip ${unreachable ? "unwired" : "wired"}">${unreachable ? "Unreachable" : "Reachable"}</span><code>${escapeHtml(outcome.id)}</code><span class="entity-degree">${inputs.length} input${inputs.length === 1 ? "" : "s"}</span></div>
        <h4>${escapeHtml(outcome.label)}</h4>
        <div class="entity-facts"><span><strong>Weight</strong> ${escapeHtml(formatNumber(outcome.weight))}</span><span><strong>Tags</strong> ${tagList(outcome.tags)}</span></div>
        <div class="entity-connections"><strong>Covering inputs</strong>${inputs.length ? entityLinks(inputs, index.inputRows, "input") : '<span class="no-connections">None in the approved profile</span>'}</div>
        ${unreachable ? '<p class="unwired-reason"><strong>Why marked unreachable:</strong> no approved relationship points to this outcome, so no selected panel can cover it. This may be a genuine gap or a projection/parser/adapter limitation.</p>' : ""}
        ${projectionSupportDetails(support, "outcome")}
      </article>` };
  });
  const relationshipRows = profile.relationships.map((relationship, position) => {
    const target = explicitProfile ? `/relationships/${position}/` : `/relationships/${escapePointerSegment(relationship.input_id)}->${escapePointerSegment(relationship.outcome_id)}`;
    const support = projectionSupport(supportIndex, target);
    const search = `${relationship.input_id} ${relationship.outcome_id}`.toLowerCase();
    return { id: `${relationship.input_id}\u0000${relationship.outcome_id}`, status: "wired", search, render: () => `<article class="relationship-row">
        <span class="relationship-index">${position + 1}</span>
        <div>${entityLink(relationship.input_id, index.inputRows.get(relationship.input_id), "input")}<span class="relationship-arrow" aria-label="covers">→</span>${entityLink(relationship.outcome_id, index.outcomeRows.get(relationship.outcome_id), "outcome")}</div>
        ${projectionSupportDetails(support, "relationship")}
      </article>` };
  });
  const inputList = registerReportList("profile-inputs", inputRows, "entity-list", "No inputs match this search and status filter.");
  const outcomeList = registerReportList("profile-outcomes", outcomeRows, "entity-list", "No outcomes match this search and status filter.");
  const relationshipList = registerReportList("profile-relationships", relationshipRows, "relationship-list", "No relationships match this search.");
  return `<section class="section-block profile-inventory" aria-labelledby="profile-inventory-title">
      <div class="section-heading"><div><p class="eyebrow">Approved optimizer model</p><h3 id="profile-inventory-title">Complete input, outcome and relationship inventory</h3></div></div>
      <p>Search uses IDs, labels, tags and linked IDs. Lists below are complete; no inventory rows are truncated.</p>
      <details class="inventory-group" id="profile-input-inventory" open>
        <summary><span>Inputs</span><strong>${profile.inputs.length}</strong><small>${index.inertInputs.size} inert</small></summary>
        ${inventoryControls("profile-inputs", "Search inputs by ID, label, tag or outcome", [["all", "All inputs"], ["wired", "Wired only"], ["inert", "Inert only"]])}
        ${inputList}
      </details>
      <details class="inventory-group" id="profile-outcome-inventory">
        <summary><span>Outcomes</span><strong>${profile.outcomes.length}</strong><small>${index.unreachableOutcomes.size} unreachable</small></summary>
        ${inventoryControls("profile-outcomes", "Search outcomes by ID, label, tag or input", [["all", "All outcomes"], ["reachable", "Reachable only"], ["unreachable", "Unreachable only"]])}
        ${outcomeList}
      </details>
      <details class="inventory-group" id="profile-relationship-inventory">
        <summary><span>Relationships</span><strong>${profile.relationships.length}</strong><small>exact input → outcome pairs</small></summary>
        ${inventoryControls("profile-relationships", "Search relationships by input or outcome ID")}
        ${relationshipList}
      </details>
    </section>`;
}

function inventoryControls(group, placeholder, statuses = []) {
  const options = statuses.map(([value, label]) => `<option value="${escapeHtml(value)}">${escapeHtml(label)}</option>`).join("");
  return `<div class="inventory-controls"><label><span class="sr-only">${escapeHtml(placeholder)}</span><input class="inventory-search" type="search" autocomplete="off" placeholder="${escapeHtml(placeholder)}" data-report-search="${escapeHtml(group)}" aria-controls="${escapeHtml(group)}"></label>${options ? `<label><span class="sr-only">Filter by wiring status</span><select class="inventory-select" data-report-status="${escapeHtml(group)}" aria-controls="${escapeHtml(group)}">${options}</select></label>` : ""}<output data-filter-count="${escapeHtml(group)}" aria-live="polite"></output></div>`;
}

function registerReportList(group, records, className, noMatchMessage) {
  const list = { group, records, className, noMatchMessage, query: "", status: "all", page: 0, pageSize: 50 };
  state.reportLists[group] = list;
  const visible = records.slice(0, list.pageSize).map((record) => record.render()).join("");
  const initial = visible || '<div class="empty-state">No entries are declared in the approved profile.</div>';
  return `<div class="${escapeHtml(className)}" id="${escapeHtml(group)}" tabindex="-1">${initial}</div><div class="filter-empty hidden" data-filter-empty="${escapeHtml(group)}">${escapeHtml(noMatchMessage)}</div><div class="list-pagination" data-report-pagination="${escapeHtml(group)}"></div>`;
}

function bindReportDrilldowns() {
  const inspector = $("#inspector");
  if (!inspector) return;
  inspector.removeEventListener("input", onReportFilterInput);
  inspector.removeEventListener("change", onReportFilterInput);
  inspector.removeEventListener("click", onReportDrilldownClick);
  inspector.addEventListener("input", onReportFilterInput);
  inspector.addEventListener("change", onReportFilterInput);
  inspector.addEventListener("click", onReportDrilldownClick);
  Object.keys(state.reportLists).forEach((group) => renderReportListPage(group));
}

function onReportFilterInput(event) {
  const group = event.target.dataset.reportSearch || event.target.dataset.reportStatus;
  if (!group || !state.reportLists[group]) return;
  const list = state.reportLists[group];
  if (event.target.dataset.reportSearch) list.query = event.target.value.trim().toLowerCase();
  if (event.target.dataset.reportStatus) list.status = event.target.value;
  list.page = 0;
  renderReportListPage(group);
}

function onReportDrilldownClick(event) {
  const pageButton = event.target.closest("[data-report-page]");
  if (pageButton) {
    const list = state.reportLists[pageButton.dataset.reportGroup];
    if (!list) return;
    list.page += pageButton.dataset.reportPage === "next" ? 1 : -1;
    renderReportListPage(list.group, true);
    return;
  }
  const reveal = event.target.closest("[data-reveal-group]");
  if (reveal) {
    revealReportRecord(reveal.dataset.revealGroup, reveal.dataset.revealId);
    return;
  }
  const jump = event.target.closest("[data-scroll-target]");
  if (jump) revealElement(jump.dataset.scrollTarget);
}

function filteredReportRecords(list) {
  return list.records.filter((record) => (!list.query || record.search.includes(list.query)) && (list.status === "all" || record.status === list.status));
}

function renderReportListPage(group, focusList = false) {
  const list = state.reportLists[group];
  const container = document.getElementById(group);
  if (!list || !container) return;
  const filtered = filteredReportRecords(list);
  const pages = Math.max(1, Math.ceil(filtered.length / list.pageSize));
  list.page = Math.max(0, Math.min(list.page, pages - 1));
  const start = list.page * list.pageSize;
  const pageRecords = filtered.slice(start, start + list.pageSize);
  container.innerHTML = pageRecords.map((record) => record.render()).join("");
  container.classList.toggle("hidden", pageRecords.length === 0);
  const empty = document.querySelector(`[data-filter-empty="${cssAttributeValue(group)}"]`);
  empty?.classList.toggle("hidden", pageRecords.length !== 0);
  const count = document.querySelector(`[data-filter-count="${cssAttributeValue(group)}"]`);
  if (count) count.textContent = filtered.length ? `Showing ${start + 1}–${start + pageRecords.length} of ${filtered.length}` : "0 matches";
  const pagination = document.querySelector(`[data-report-pagination="${cssAttributeValue(group)}"]`);
  if (pagination) pagination.innerHTML = filtered.length > list.pageSize ? `<button type="button" data-report-page="previous" data-report-group="${escapeHtml(group)}" ${list.page === 0 ? "disabled" : ""}>← Previous</button><span>Page ${list.page + 1} of ${pages}</span><button type="button" data-report-page="next" data-report-group="${escapeHtml(group)}" ${list.page + 1 >= pages ? "disabled" : ""}>Next →</button>` : "";
  if (focusList) container.focus?.({ preventScroll: true });
}

function revealReportRecord(group, id) {
  const list = state.reportLists[group];
  if (!list) return;
  list.query = "";
  list.status = "all";
  const position = list.records.findIndex((record) => record.id === id);
  if (position < 0) return;
  list.page = Math.floor(position / list.pageSize);
  const search = document.querySelector(`[data-report-search="${cssAttributeValue(group)}"]`);
  const status = document.querySelector(`[data-report-status="${cssAttributeValue(group)}"]`);
  if (search) search.value = "";
  if (status) status.value = "all";
  renderReportListPage(group);
  const container = document.getElementById(group);
  const disclosure = container?.closest("details");
  if (disclosure) disclosure.open = true;
  requestAnimationFrame(() => revealElement(group === "profile-inputs" ? buildProfileRowId("input", position) : buildProfileRowId("outcome", position)));
}

function buildProfileRowId(kind, position) { return `profile-${kind}-${position}`; }

function revealElement(id) {
  const target = document.getElementById(id);
  if (!target) return;
  const disclosure = target.closest("details");
  if (disclosure) disclosure.open = true;
  target.scrollIntoView({ behavior: "smooth", block: "center" });
  target.focus?.({ preventScroll: true });
}

function cssAttributeValue(value) {
  return String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

function buildProjectionSupportIndex(candidate) {
  const groups = new Map();
  const bucket = (key) => {
    if (!groups.has(key)) groups.set(key, { evidence: [], assumptions: [], diagnostics: [] });
    return groups.get(key);
  };
  (candidate.evidence || []).forEach((item) => bucket(projectionTargetGroup(item.target_field)).evidence.push(item));
  (candidate.assumptions || []).forEach((item) => bucket(projectionTargetGroup(item.target_field)).assumptions.push(item));
  (candidate.diagnostics || []).forEach((item) => {
    const targets = new Set((item.evidence || []).map((entry) => projectionTargetGroup(entry.target_field)));
    targets.forEach((target) => bucket(target).diagnostics.push(item));
  });
  return groups;
}

function projectionTargetGroup(target) {
  const value = String(target || "");
  const entity = value.match(/^\/(inputs|outcomes)\/([^/]+)\//);
  if (entity) return `/${entity[1]}/${entity[2]}/`;
  const indexedRelationship = value.match(/^\/relationships\/(\d+)\//);
  if (indexedRelationship) return `/relationships/${indexedRelationship[1]}/`;
  return value;
}

function projectionSupport(index, target) {
  return index.get(target) || { evidence: [], assumptions: [], diagnostics: [] };
}

function projectionSupportDetails(support, kind) {
  const evidence = support.evidence.map((item) => `<li><code>${escapeHtml(item.path)}${escapeHtml(item.pointer || "")}</code><span>supports <code>${escapeHtml(item.target_field)}</code> · artifact ${escapeHtml(shortHash(item.artifact_id))}</span></li>`).join("");
  const assumptions = support.assumptions.map((item) => `<li><span class="support-kind assumed">Assumption</span><strong>${escapeHtml(item.code)}</strong><span>${escapeHtml(item.message)} · value <code>${escapeHtml(item.value)}</code></span></li>`).join("");
  const diagnostics = support.diagnostics.map((item) => `<li><span class="support-kind diagnostic">${escapeHtml(item.severity || "diagnostic")}</span><strong>${escapeHtml(item.code)}</strong><span>${escapeHtml(item.message)}</span></li>`).join("");
  const total = support.evidence.length + support.assumptions.length + support.diagnostics.length;
  return `<details class="projection-support"><summary>${support.evidence.length} evidence link${support.evidence.length === 1 ? "" : "s"} · ${support.assumptions.length} assumption${support.assumptions.length === 1 ? "" : "s"}${support.diagnostics.length ? ` · ${support.diagnostics.length} diagnostic${support.diagnostics.length === 1 ? "" : "s"}` : ""}</summary>${total ? `<ul>${evidence}${assumptions}${diagnostics}</ul>` : `<p>No field-level projection support was recorded for this ${escapeHtml(kind)}. Review the approved profile and source coverage.</p>`}</details>`;
}

function projectionReviewLedger(candidate) {
  if (!(candidate.assumptions || []).length && !(candidate.diagnostics || []).length) return "";
  const assumptions = (candidate.assumptions || []).map((item) => `<li><span class="support-kind assumed">Assumption</span><strong><code>${escapeHtml(item.target_field)}</code> · ${escapeHtml(item.code)}</strong><p>${escapeHtml(item.message)} Value: <code>${escapeHtml(item.value)}</code>.</p></li>`).join("");
  const diagnostics = (candidate.diagnostics || []).map((item) => `<li><span class="support-kind diagnostic">${escapeHtml(item.severity)}</span><strong>${escapeHtml(item.code)}</strong><p>${escapeHtml(item.message)}</p></li>`).join("");
  return `<details class="limitations projection-ledger"><summary>All projection assumptions (${candidate.assumptions.length}) and diagnostics (${candidate.diagnostics.length})</summary><p>No records are truncated in this ledger.</p><ul>${diagnostics}${assumptions}</ul></details>`;
}

function entityLinks(ids, rowMap, kind) {
  return `<span class="entity-links">${ids.map((id) => entityLink(id, rowMap.get(id), kind)).join("")}</span>`;
}

function entityLink(id, target, kind) {
  const group = kind === "input" ? "profile-inputs" : "profile-outcomes";
  return `<button class="entity-link" type="button" data-reveal-group="${group}" data-reveal-id="${escapeHtml(id)}" data-scroll-target="${escapeHtml(target || "")}" title="Show ${escapeHtml(kind)} ${escapeHtml(id)}"><code>${escapeHtml(id)}</code></button>`;
}

function tagList(tags) {
  return tags?.length ? `<span class="tag-list">${tags.map((tag) => `<span>${escapeHtml(tag)}</span>`).join("")}</span>` : '<span class="muted-value">None declared</span>';
}

function escapePointerSegment(value) {
  return String(value).replaceAll("~", "~0").replaceAll("/", "~1");
}

function comparisonSummary(result) {
  const x = capabilityValue(result);
  if (!x) return skippedSummary(result, "Version comparison unavailable");
  if (!x.semantic_diff) return '<div class="empty-state">No earlier approved profile with the same stable profile identity exists. This approved profile becomes the baseline for its next version.</div>';
  const d = x.semantic_diff;
  const changes = [
    ["Inputs", `+${d.inputs_added.length} / −${d.inputs_removed.length}`, `${d.inputs_modified.length} modified`],
    ["Outcomes", `+${d.outcomes_added.length} / −${d.outcomes_removed.length}`, `${d.outcomes_modified.length} modified`],
    ["Relationships", `+${d.relationships_added.length} / −${d.relationships_removed.length}`, "Binary incidence changes"],
    ["Reachability", `${d.newly_unreachable_outcomes.length} newly unreachable`, `${d.newly_reachable_outcomes.length} newly reachable`],
    ["Input activity", `${d.newly_active_inputs.length} newly active`, `${d.newly_inert_inputs.length} newly inert`],
    ["Constraints", d.constraints_changed ? "Changed" : "Unchanged", d.objective_changed ? "Objective changed" : "Objective unchanged"],
  ];
  const coverage = (x.coverage_changes || []).map((point) => dataRow(`K=${point.k}`, point.after_fraction == null ? "N/A" : formatRatio(point.after_fraction), point.change == null ? "No comparable baseline" : signedPercent(point.change))).join("");
  const panels = (x.minimum_panel_changes || []).map((point) => dataRow(`${Math.round(point.coverage_floor * 100)}% coverage`, point.after_size ?? "N/A", point.change == null ? point.after_reason || "Not comparable" : signedNumber(point.change))).join("");
  return `<section class="section-block"><h3>Semantic changes</h3><div class="data-list">${changes.map((item) => dataRow(item[0], item[1], item[2])).join("")}</div></section><section class="split-results"><div><h3>Coverage change at K</h3><div class="data-list">${coverage || '<div class="empty-state">No shared panel size could be compared.</div>'}</div></div><div><h3>Minimum panel change</h3><div class="data-list">${panels}</div></div></section>${quboDeltaSummary(x.qubo_change)}`;
}

function optimizationSummary(result, workflow = {}) {
  const x = capabilityValue(result);
  if (!x) return skippedSummary(result, "Classical optimization unavailable");
  const profile = capabilityValue(workflow.biomedical_profile)?.profile;
  const index = profile ? buildProfileIndex(profile) : null;
  const coverage = (x.coverage_at_k || []).map((point) => coveragePanel(point, x.structural, profile, index)).join("");
  const panels = (x.minimum_panels || []).map((point) => dataRow(`${Math.round(point.coverage_floor * 100)}% floor`, point.result.value?.score?.selected_count ?? "Unavailable", point.result.value ? `${point.result.value.optimality_proven ? "Proven" : "Baseline"} · cost ${formatNumber(point.result.value.score.total_cost)}` : point.result.reason)).join("");
  const reachabilityCeiling = x.structural.total_outcome_weight > 0 ? x.structural.reachable_outcome_weight / x.structural.total_outcome_weight : 0;
  const st = x.structural;
  const vetoOnly = (st.veto_only_inputs || []).length;
  const unconditional = (st.unconditional_outcomes || []).length;
  return `<div class="metric-grid">${metric(st.input_count, "Inputs")}${metric(st.outcome_count, "Outcomes")}${metric(st.inert_inputs.length, "Inert inputs")}${metric(st.unreachable_outcomes.length, "Unreachable outcomes")}</div>
    ${ruleShapeNotice(st)}
    ${vetoOnly || unconditional ? `<section class="section-block"><h3>Other structural findings</h3><div class="data-list">${vetoOnly ? dataRow("Veto-only inputs", vetoOnly, "Mentioned by the rules only to block an outcome or supply context, so selecting one can never grant coverage. Different from inert, which means no rule mentions it at all.") : ""}${unconditional ? dataRow("Unconditional outcomes", unconditional, "Covered by every panel including the empty one. Counted separately because they inflate any coverage ratio without any input earning them.") : ""}</div></section>` : ""}
    <div class="notice-strip reachability-notice"><strong>Structural coverage ceiling: at most ${formatRatio(reachabilityCeiling)}</strong><span>${formatNumber(x.structural.reachable_outcome_weight)} of ${formatNumber(x.structural.total_outcome_weight)} total outcome weight can be covered by some panel. The rest is out of reach for every selection, usually because an outcome's rule arms can never be satisfied together. No optimizer can exceed this bound; profile constraints can lower it further.</span></div>
    <section class="section-block coverage-results" id="coverage-panel-results" tabindex="-1"><div class="section-heading"><div><p class="eyebrow">Deterministic baseline</p><h3>Complete coverage results at K</h3></div><span class="coverage-score">K is a ceiling</span></div><p><strong>K=5 means “select at most five inputs”; K=10 means “select at most ten.”</strong> It does not request five or ten outcomes, and the returned panel can contain fewer than K inputs. Coverage percentage is weighted by outcome importance; the covered-outcome count is shown separately.</p><div class="coverage-panel-list">${coverage || '<div class="empty-state">No K coverage points were measured.</div>'}</div></section>
    <section class="section-block"><h3>Smallest measured panels</h3><div class="data-list">${panels}</div></section>
    <section class="section-block"><h3>Solver comparison</h3><div class="data-list">${(x.solver_results || []).map((item) => item.value ? dataRow(item.value.solver, `${formatRatio(item.value.score.coverage_fraction)} weighted coverage`, `${item.value.score.selected_count} inputs · ${item.value.optimality_proven ? "optimality proven" : "no proof"}`) : dataRow("Unavailable solver", "Skipped", item.reason)).join("")}</div></section>
    ${(x.limitations || []).length ? `<details class="limitations"><summary>Optimization-wide limitations (${x.limitations.length})</summary><ul>${x.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></details>` : ""}`;
}

// Whether coverage on this profile is a submodular problem, and what follows.
function ruleShapeNotice(structural) {
  const arms = structural.arm_count;
  if (arms == null) return "";
  const conjunctive = structural.conjunctive_arm_count || 0;
  if (structural.purely_disjunctive) {
    return `<div class="notice-strip"><strong>Every outcome is covered by a single input.</strong><span>${arms} rule arms, none needing a combination. Weighted coverage is submodular here, so the greedy baseline carries the classical 1−1/e guarantee and usually lands on or near the certified answer.</span></div>`;
  }
  return `<div class="notice-strip express-notice"><strong>${conjunctive} of ${arms} rule arms need several inputs together.</strong><span>Coverage is not submodular on this profile: adding an input can remove coverage, and greedy carries no approximation guarantee. This is the property that makes the choice of optimizer change the answer, which is why the certified solver is worth its runtime here.</span></div>`;
}

function coveragePanel(point, structural, profile, index) {
  const result = point.result;
  const score = result.score;
  const open = point.k === 5 || point.k === 10 ? " open" : "";
  const underCeiling = score.selected_count < point.k ? `<p class="panel-note"><strong>${score.selected_count} selected, not ${point.k}:</strong> K is only a maximum. Unused capacity is not an error; this solver did not return additional allowed inputs that improved its scored objective.</p>` : "";
  const request = result.request || {};
  const profileConstraints = profile?.constraints;
  const constraintText = score.profile_constraints_satisfied ? "Satisfied" : `${score.constraint_violations.length} violation${score.constraint_violations.length === 1 ? "" : "s"}`;
  return `<details class="coverage-panel"${open}>
      <summary><span><strong>K=${point.k}</strong><small>maximum inputs</small></span><span><strong>${formatRatio(score.coverage_fraction)}</strong><small>weighted coverage</small></span><span><strong>${score.selected_count}</strong><small>selected</small></span><span><strong>${score.covered_outcomes.length}/${structural.outcome_count}</strong><small>outcomes covered</small></span></summary>
      <div class="coverage-panel-body">${underCeiling}<div class="panel-metrics">${dataRow("Selected inputs", `${score.selected_count} of at most ${point.k}`, `cost ${formatNumber(score.total_cost)}`)}${dataRow("Weighted coverage", formatRatio(score.coverage_fraction), `${formatNumber(score.covered_weight)} of ${formatNumber(score.total_outcome_weight)} outcome weight`)}${dataRow("Outcome count", `${score.covered_outcomes.length} covered`, `${score.uncovered_outcomes.length} uncovered`)}${dataRow("Solver", statusLabel(result.solver), result.optimality_proven ? "Optimality proven" : "Baseline result; no proof of optimality")}${dataRow("Reproducibility", result.deterministic_seed ?? "No seed used", `${result.iterations_completed} iterations · ${result.evaluated_candidates} candidates`)}${dataRow("Constraints", constraintText, optimizationConstraintNote(request, profileConstraints))}</div>
        ${score.constraint_violations.length ? `<div class="inline-alert"><strong>Constraint violations</strong><ul>${score.constraint_violations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></div>` : ""}
        ${panelIdCollection(`coverage-k-${point.k}-selected`, "Selected inputs", score.selected_inputs, "input", index, true)}
        ${panelIdCollection(`coverage-k-${point.k}-covered`, "Covered outcomes", score.covered_outcomes, "outcome", index)}
        ${panelIdCollection(`coverage-k-${point.k}-uncovered`, "Uncovered outcomes", score.uncovered_outcomes, "outcome", index)}
        ${(result.limitations || []).length ? `<details class="limitations"><summary>Solver limitations (${result.limitations.length})</summary><ul>${result.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></details>` : ""}
      </div>
    </details>`;
}

function panelIdCollection(group, label, ids, kind, index, open = false) {
  const mainGroup = kind === "input" ? "profile-inputs" : "profile-outcomes";
  const rowMap = kind === "input" ? index?.inputRows : index?.outcomeRows;
  const linkable = Boolean(state.reportLists[mainGroup]);
  const records = ids.map((id) => ({
    id,
    status: "all",
    search: String(id).toLowerCase(),
    render: () => linkable ? entityLink(id, rowMap?.get(id), kind) : `<span class="entity-token"><code>${escapeHtml(id)}</code></span>`,
  }));
  const list = registerReportList(group, records, "panel-id-list", `No ${label.toLowerCase()} match this search.`);
  return `<details class="panel-id-group"${open ? " open" : ""}><summary>${escapeHtml(label)} <strong>${ids.length}</strong></summary>${inventoryControls(group, `Search ${label.toLowerCase()} by ID`)}${list}</details>`;
}

function optimizationConstraintNote(request, constraints) {
  const parts = [`request max_inputs=${request.max_inputs ?? "none"}`, `coverage_floor=${request.coverage_floor ?? "none"}`];
  if (constraints) {
    parts.push(`profile min=${constraints.min_selected}`);
    parts.push(`profile max=${constraints.max_selected ?? "none"}`);
    parts.push(`cost cap=${constraints.max_total_cost ?? "none"}`);
    parts.push(`${(constraints.required_inputs || []).length} required input(s)`);
    parts.push(`${(constraints.excluded_inputs || []).length} excluded input(s)`);
    parts.push(`${(constraints.required_outcomes || []).length} required outcome(s)`);
  }
  return parts.join(" · ");
}

function quboSummary(result) {
  const x = capabilityValue(result);
  if (!x) return skippedSummary(result, "QUBO/Ising validation unavailable");
  const m = x.metrics || {}; const d = x.difficulty || {}; const v = x.energy_validation || {};
  const exportOnly = (x.execution?.status || "not_requested") === "export_only";
  const checked = v.assignments_checked || 0;
  const exhaustive = v.exhaustive === true;
  // 2^n grows past anything worth printing, so say how many were checked and
  // let the "spot check" label carry the meaning rather than a huge number.
  const coverageNote = exhaustive
    ? `every assignment checked · max error ${formatNumber(v.maximum_absolute_error || 0)}`
    : `spot check of ${checked.toLocaleString()} assignments out of 2^${m.variable_count ?? "?"} · max error ${formatNumber(v.maximum_absolute_error || 0)}`;
  return `<div class="metric-grid">${metric(m.variable_count ?? "—", "Logical variables")}${metric(m.input_variable_count ?? "—", "Input variables")}${metric(m.arm_variable_count ?? 0, "Arm variables")}${metric(m.slack_variable_count ?? "—", "Slack variables")}</div>
    ${exportOnly ? '<div class="notice-strip express-notice"><strong>Nothing was run on a quantum computer.</strong><span>This stage builds the QUBO and Ising models and checks that they agree, then exports them. Q-BenchMed contains no provider client and submits no job. Running the formulation is a separate, separately reviewed step.</span></div>' : ""}
    <section class="section-block"><h3>Validation and execution boundary</h3><div class="data-list">${dataRow("QUBO → Ising", v.maximum_absolute_error === 0 ? "Energies agree" : "Energies differ", coverageNote)}${exhaustive ? "" : dataRow("What that proves", "Agreement on what was checked", "A bounded sample is evidence, not a proof of equivalence over the whole spectrum. The limit and the sample size are recorded so the check can be repeated or widened.")}${dataRow("Quantum execution", statusLabel(x.execution?.status || "not_requested"), x.execution?.summary || "Optional provider execution is separate from formulation validation")}${dataRow("Structural difficulty", `${d.score ?? "—"}/100 · ${d.level || "unavailable"}`, d.disclaimer || "Not a runtime prediction")}</div></section>
    ${(x.limitations || []).length ? `<details class="limitations"><summary>Formulation limitations (${x.limitations.length})</summary><ul>${x.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></details>` : ""}`;
}

function skippedSummary(result, fallback) { return `<div class="empty-state"><strong>${escapeHtml(fallback)}</strong><br>${escapeHtml(result?.skip_reason || "The required approved semantic input was not available.")}</div>${result?.limitations?.length ? `<details class="limitations" open><summary>Recorded limitations</summary><ul>${result.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></details>` : ""}`; }
function quboDeltaSummary(delta) { if (!delta) return ""; return `<section class="section-block"><h3>QUBO and difficulty change</h3><div class="data-list">${dataRow("Logical variables", delta.after.value?.metrics?.variable_count ?? "N/A", delta.variable_count_change == null ? delta.after.reason || "Not comparable" : signedNumber(delta.variable_count_change))}${dataRow("Couplings", delta.after.value?.metrics?.coupling_count ?? "N/A", delta.coupling_count_change == null ? delta.after.reason || "Not comparable" : signedNumber(delta.coupling_count_change))}${dataRow("Difficulty score", delta.after.value?.difficulty?.score ?? "N/A", delta.difficulty_score_change == null ? delta.after.reason || "Not comparable" : signedNumber(delta.difficulty_score_change))}</div></section>`; }

function parserCoverage(parser) {
  const formatRows = parser.formats.map((item) => `<div class="coverage-row"><span><strong>${escapeHtml(item.format)}</strong><small>${escapeHtml(item.status.replaceAll("_", " "))}</small></span><span>${escapeHtml(String(item.parsed_files))} / ${escapeHtml(String(item.files))} files</span><span>${formatPercent(item.files ? Math.round(item.parsed_files * 10000 / item.files) : 10000)}</span></div>`).join("");
  return `<section class="section-block"><div class="section-heading"><div><p class="eyebrow">Measured scope</p><h3>Parser coverage</h3></div><span class="coverage-score">${formatPercent(parser.file_coverage_basis_points)}</span></div><p>${escapeHtml(String(parser.parsed_files))} of ${escapeHtml(String(parser.total_files))} files and ${escapeHtml(formatBytes(parser.parsed_bytes))} of ${escapeHtml(formatBytes(parser.total_bytes))} received structured parsing. Unsupported files remain immutable evidence, but Q-BenchMed does not claim to understand their contents.</p><div class="coverage-list">${formatRows}</div></section>`;
}

function languageScan(scan) {
  if (!scan) return "";
  const rows = [
    ["Rust", `${scan.rust_modules} modules · ${scan.rust_items} items`, `${scan.rust_imports} imports`],
    ["Python", `${scan.python_modules} modules · ${scan.python_classes} classes · ${scan.python_functions} functions`, `${scan.python_tests} tests · ${scan.python_calls} calls · ${scan.python_imports} imports`],
    ["Dependencies", scan.dependency_count, (scan.dependency_samples || []).slice(0, 12).join(", ") || "None observed"],
  ];
  return `<section class="section-block"><h3>Static Rust and Python structure</h3><div class="data-list">${rows.map((item) => dataRow(item[0], item[1], item[2])).join("")}</div><details class="limitations"><summary>Scanner limitations</summary><ul>${scan.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></details></section>`;
}

function checkLedger(coverage) {
  const rows = coverage.checks.map((check) => `<article class="check-row"><span class="check-status ${escapeHtml(check.status)}">${escapeHtml(check.status.replaceAll("_", " "))}</span><div><strong>${escapeHtml(check.check_id)}</strong><p>${escapeHtml(check.explanation)}</p>${check.skip_reason ? `<small>${escapeHtml(check.skip_reason)}</small>` : ""}</div><span class="check-count">${escapeHtml(String(check.findings))}</span></article>`).join("");
  const limitations = coverage.limitations.map((item) => `<li>${escapeHtml(item)}</li>`).join("");
  return `<section class="section-block"><div class="section-heading"><div><p class="eyebrow">${escapeHtml(coverage.completion_level.replaceAll("_", " "))}</p><h3>Checks that ran—and checks that did not</h3></div></div><div class="check-ledger">${rows}</div><details class="limitations" open><summary>Known limitations</summary><ul>${limitations}</ul></details></section>`;
}

function formatPercent(basisPoints) { return `${(Number(basisPoints) / 100).toFixed(2)}%`; }
function formatRatio(value) { const number = Number(value); return Number.isFinite(number) ? `${(number * 100).toFixed(2)}%` : "N/A"; }
function formatNumber(value) { const number = Number(value); return Number.isFinite(number) ? number.toLocaleString(undefined, { maximumFractionDigits: 4 }) : String(value ?? "N/A"); }
function signedNumber(value) { const number = Number(value); return Number.isFinite(number) ? `${number > 0 ? "+" : ""}${number}` : "N/A"; }
function signedPercent(value) { const number = Number(value); return Number.isFinite(number) ? `${number > 0 ? "+" : ""}${(number * 100).toFixed(2)} percentage points` : "N/A"; }

function renderRecent(runs) {
  if (!runs?.length) return; $("#recent-section").classList.remove("hidden");
  $("#recent-runs").innerHTML = runs.slice(0, 6).map((run) => `<a class="recent-card" href="#run/${escapeHtml(run.run_id)}"><strong>${escapeHtml(run.project_name)}</strong><small>${sourceKindLabel(run.source_kind)} · ${formatDate(run.updated_at)}</small><span class="status-line">${statusLabel(run.state)} →</span></a>`).join("");
}

async function api(url, options = {}) {
  const init = { method: options.method || "GET", headers: { Accept: "application/json" } };
  if (init.method !== "GET") init.headers["X-QBM-CSRF"] = state.bootstrap?.csrf_token || "";
  if (options.json) { init.headers["Content-Type"] = "application/json"; init.body = JSON.stringify(options.json); } else if (options.body) init.body = options.body;
  let response;
  try { response = await fetch(url, init); } catch (_) { const error = new Error("Disconnected from the local Q-BenchMed service. Reconnect and try again."); error.code = "disconnected"; throw error; }
  if (!response.ok) { let body = null; try { body = await response.json(); } catch (_) { /* non-JSON boundary error */ } const error = new Error(body?.error?.message || `The local service returned HTTP ${response.status}.`); error.code = body?.error?.code || "http_error"; throw error; }
  return response.json();
}

function setView(view) { $("#home-view").classList.toggle("hidden", view !== "home"); $("#processing-view").classList.toggle("hidden", view !== "processing"); $("#workspace-view").classList.toggle("hidden", view !== "workspace"); if (view !== "workspace") $("#new-audit").classList.add("hidden"); window.scrollTo({ top: 0, behavior: "smooth" }); }
function showHome() { history.pushState(null, "", location.pathname); state.workflow = null; setView("home"); $("#home-title").focus?.(); }
function showHomeError(message) { const node = $("#home-error"); node.textContent = message; node.classList.remove("hidden"); requestAnimationFrame(() => node.focus()); }
function hideHomeError() { $("#home-error").classList.add("hidden"); }
function showInspectorError(message) { const header = $(".inspector-header"); if (!header) return showToast(message); const old = $("#inspector-error"); if (old) old.remove(); const node = document.createElement("div"); node.id = "inspector-error"; node.className = "inline-alert"; node.setAttribute("role", "alert"); node.tabIndex = -1; node.textContent = message; header.appendChild(node); node.focus(); }
function disableActions(disabled) { $$('[data-decision], [data-advance]').forEach((button) => { button.disabled = disabled; }); }
function showToast(message) { const node = $("#toast"); node.textContent = message; node.classList.remove("hidden"); clearTimeout(showToast.timer); showToast.timer = setTimeout(() => node.classList.add("hidden"), 2400); }
async function copyText(value, message) { if (!value) return; try { await navigator.clipboard.writeText(value); showToast(message); } catch (_) { showToast("Copy was unavailable in this browser."); } }
function shortHash(value) { return value ? `${value.slice(0, 10)}…${value.slice(-6)}` : ""; }
function formatBytes(bytes) { const value = Number(bytes); if (!Number.isFinite(value)) return String(bytes); if (value < 1024) return `${value} B`; const units = ["KiB", "MiB", "GiB", "TiB"]; let amount = value; let unit = "B"; for (const next of units) { amount /= 1024; unit = next; if (amount < 1024) break; } return `${amount.toFixed(amount >= 10 ? 1 : 2)} ${unit}`; }
function formatDate(value) { try { return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(value)); } catch (_) { return value; } }
function sourceKindLabel(value) { return ({ uploaded_archive: "Uploaded archive", uploaded_folder: "Uploaded folder", public_github: "Public GitHub commit" })[value] || "Managed source"; }
function statusLabel(value) { return ({ not_started: "Not started", not_applicable: "Not applicable", skipped: "Skipped", not_requested: "Not requested", not_configured: "Not configured", no_baseline: "No baseline", export_only: "Export only", running: "Running", waiting_approval: "Awaiting review", approved: "Approved", needs_changes: "Changes requested", rejected: "Rejected", complete: "Complete", failed: "Failed" })[value] || String(value || "Unknown").replaceAll("_", " "); }
function escapeHtml(value) { return String(value ?? "").replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char]); }
