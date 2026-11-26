const form = document.querySelector("form");
const el = id => document.getElementById(id);
const labels = {"report.html": "Download HTML report", "orders.csv": "Download order ledger", "manifest.json": "Download replay manifest"};
let active = new URLSearchParams(location.search).get("run");
let timer;
let generation = 0;

async function request(url, options) {
  const response = await fetch(url, options);
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    const detail = data.detail;
    throw new Error(Array.isArray(detail) ? detail.map(x => x.loc.slice(1).join(".") + ": " + x.msg).join("\n") : detail || "HTTP " + response.status);
  }
  return response.json();
}

async function refresh() {
  const current = ++generation;
  clearTimeout(timer);
  el("retry").hidden = true;
  try {
    const run = await request("/v1/runs/" + encodeURIComponent(active));
    if (current !== generation) return;
    el("state").textContent = run.state;
    el("metadata").replaceChildren();
    for (const [label, value] of [["Run ID",run.id], ["Strategy",run.options.strategy], ["Initial capital",run.options.capital]]) {
      const key = document.createElement("dt"), item = document.createElement("dd");
      key.textContent = label;
      item.textContent = value;
      el("metadata").append(key, item);
    }
    el("diagnostic").textContent = run.error || "";
    el("artifacts").replaceChildren();
    for (const [name, url] of Object.entries(run.artifacts)) {
      const link = document.createElement("a");
      link.href = url;
      link.textContent = labels[name] || name;
      el("artifacts").append(link);
    }
    if (["queued","running"].includes(run.state)) timer = setTimeout(refresh, 1000);
  } catch (err) {
    if (current !== generation) return;
    el("diagnostic").textContent = err.message;
    el("retry").hidden = false;
  }
}

el("retry").onclick = refresh;
if (active) refresh();

async function setup() {
  const schema = await request("/v1/run-options");
  const strategy = form.elements.strategy;
  strategy.replaceChildren();
  for (const name of schema.properties.strategy.enum) {
    strategy.add(new Option(name === "all" ? "Compare all strategies" : name, name));
  }
  for (const [name, field] of Object.entries(schema.properties)) {
    const input = form.elements.namedItem(name);
    if (!input || field.default == null) continue;
    if (input.type === "checkbox") input.checked = field.default;
    else input.value = field.default;
  }
  el("start").disabled = false;
}

form.onsubmit = async event => {
  event.preventDefault();
  const file = el("file").files[0];
  el("error").textContent = "";
  if (!file || file.size === 0 || file.size > 8 * 1024 * 1024) {
    el("error").textContent = "Choose a non-empty CSV up to 8 MiB.";
    return;
  }
  const options = {strategy: form.elements.strategy.value, limit_includes_fees: form.elements.limit_includes_fees.checked};
  for (const input of form.querySelectorAll('input[type="number"]')) {
    if (input.value !== "") options[input.name] = input.valueAsNumber;
  }
  el("start").disabled = true;
  try {
    const dataset = await request("/v1/datasets", {method:"POST", headers:{"Content-Type":"text/csv"}, body:file});
    const run = await request("/v1/runs", {method:"POST", headers:{"Content-Type":"application/json"}, body:JSON.stringify({dataset:dataset.id, options})});
    active = run.id;
    const url = new URL(location.href);
    url.searchParams.set("run", active);
    history.replaceState(null,"",url);
    el("metadata").replaceChildren();
    el("artifacts").replaceChildren();
    el("diagnostic").textContent = "";
    el("state").textContent = run.state;
    await refresh();
  } catch (err) {
    el("error").textContent = err.message;
  } finally {
    el("start").disabled = false;
  }
};
setup().catch(err => { el("error").textContent = "Could not load run options. Reload to retry. " + err.message; });
