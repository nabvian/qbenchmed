// Runs the unmodified qbm package in Pyodide, off the main thread.
const PYODIDE_URL = "https://cdn.jsdelivr.net/pyodide/v0.29.3/full/";
const ALLOWED = new Set(["prepare", "run_step", "constrained_comparison", "heme_curve"]);

importScripts(`${PYODIDE_URL}pyodide.js`);

const status = (text) => postMessage({ type: "status", text });

const ready = (async () => {
  status("Loading the Python runtime…");
  const pyodide = await loadPyodide({ indexURL: PYODIDE_URL });
  status("Loading numpy and scipy…");
  await pyodide.loadPackage(["numpy", "scipy", "pyyaml"]);
  status("Loading the qbm package…");
  const archive = await fetch(new URL("qbm.zip", self.location.href));
  if (!archive.ok) throw new Error(`qbm.zip: HTTP ${archive.status}`);
  pyodide.unpackArchive(await archive.arrayBuffer(), "zip", { extractDir: "/home/pyodide/qbm_site" });
  pyodide.runPython("import sys, json\nsys.path.insert(0, '/home/pyodide/qbm_site')\nimport qbm_web");
  return pyodide;
})();

ready.then(
  () => postMessage({ type: "ready" }),
  (error) => postMessage({ type: "failed", error: String(error?.message ?? error) }),
);

self.onmessage = async ({ data }) => {
  const { id, fn, args } = data;
  try {
    if (!ALLOWED.has(fn)) throw new Error(`unknown function ${fn}`);
    const pyodide = await ready;
    pyodide.globals.set("_call_args", JSON.stringify(args ?? []));
    const result = pyodide.runPython(`json.dumps(qbm_web.${fn}(*json.loads(_call_args)))`);
    postMessage({ type: "result", id, ok: true, result: JSON.parse(result) });
  } catch (error) {
    const message = String(error?.message ?? error).trim().split("\n").filter(Boolean).at(-1) ?? "Python error";
    postMessage({ type: "result", id, ok: false, error: message });
  }
};
