import {createComparison} from "./comparison.js";

const form = document.querySelector("form");
const el = id => document.getElementById(id);
const comparison = createComparison(el("comparison"));
const labels = {"report.html": "Download HTML report", "summary.json": "Download portfolio summary", "equity.json": "Download equity curves", "orders.csv": "Download order ledger", "manifest.json": "Download replay manifest"};
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

let nextCursor = null;
let historyRequest = 0;

async function loadHistory(append = false) {
  const token = ++historyRequest;
  const cursor = append ? nextCursor : null;
  const list = el("past-runs");
  el("older").disabled = true;
  try {
    const page = await request("/v1/runs" + (cursor ? "?before=" + encodeURIComponent(cursor) : ""));
    if (token !== historyRequest) return;
    if (!append) list.replaceChildren();
    for (const run of page.runs) {
      const row = document.createElement("li");
      row.dataset.id = run.id;
      const link = document.createElement("a");
      link.href = "?run=" + encodeURIComponent(run.id);
      link.textContent = (run.strategy === "all" ? "All strategies" : run.strategy) + " · " + run.id.slice(0, 8);
      const details = document.createElement("span");
      const daily = run.daily_limit == null ? "" : " · daily limit " + run.daily_limit;
      details.textContent = new Date(run.created_at).toLocaleString() + " · capital " + run.capital + daily;
      const state = document.createElement("span");
      state.className = "run-state";
      state.textContent = run.state;
      row.append(link, details, state);
      list.append(row);
    }
    nextCursor = page.next;
    el("older").hidden = !nextCursor;
    el("history-message").textContent = list.children.length ? "" : "No runs yet.";
  } catch (err) {
    if (token === historyRequest) el("history-message").textContent = "Could not load history. " + err.message;
  } finally {
    if (token === historyRequest) el("older").disabled = false;
  }
}

el("history-refresh").onclick = () => loadHistory();
el("older").onclick = () => loadHistory(true);
loadHistory();

async function refresh() {
  const current = ++generation;
  clearTimeout(timer);
  el("retry").hidden = true;
  try {
    const run = await request("/v1/runs/" + encodeURIComponent(active));
    if (current !== generation) return;
    el("state").textContent = run.state;
    const rowState = el("past-runs").querySelector(`[data-id="${run.id}"] .run-state`);
    if (rowState) rowState.textContent = run.state;
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
    void comparison.show(run);
    const cancellable = ["queued","running"].includes(run.state);
    el("cancel-run").hidden = !cancellable;
    if (cancellable) timer = setTimeout(refresh, 1000);
  } catch (err) {
    if (current !== generation) return;
    el("diagnostic").textContent = err.message;
    el("retry").hidden = false;
  }
}

el("cancel-run").onclick = async () => {
  const id = active;
  el("cancel-run").disabled = true;
  el("cancel-error").textContent = "";
  try {
    await request("/v1/runs/" + encodeURIComponent(id) + "/cancel", {method:"POST"});
  } catch (err) {
    if (id === active) el("cancel-error").textContent = err.message;
  } finally {
    if (id === active) await refresh();
    el("cancel-run").disabled = false;
  }
};
el("retry").onclick = refresh;
if (active) refresh();

function addStrategyParameter(parameter) {
  const input = document.createElement("input");
  input.type = "number";
  input.name = parameter.id;
  input.step = parameter.type === "integer" ? "1" : "any";
  input.required = parameter.default !== null;
  if (parameter.default === null) input.placeholder = "Strategy default";
  else input.value = parameter.default;
  if (parameter.minimum !== undefined) input.min = parameter.minimum;
  if (parameter.maximum !== undefined) input.max = parameter.maximum;
  input.addEventListener("input", () => {
    const value = input.valueAsNumber;
    const minimum = parameter.exclusive_minimum;
    const maximum = parameter.exclusive_maximum;
    input.setCustomValidity(
      value <= minimum ? "Enter a value greater than " + minimum + "." :
      value >= maximum ? "Enter a value less than " + maximum + "." : ""
    );
  });
  const label = document.createElement("label");
  label.title = parameter.description;
  label.append(parameter.label, input);
  const field = document.createElement("div");
  field.append(label);
  el(parameter.advanced ? "strategy-settings" : "strategy-primary").append(field);
  return {field, input};
}

async function setup() {
  const schema = await request("/v1/run-options");
  const catalog = schema.strategy_catalog;
  const strategy = form.elements.strategy;
  strategy.replaceChildren();
  for (const {id} of catalog.strategies) {
    strategy.add(new Option(id, id));
  }
  strategy.add(new Option("Compare all strategies", catalog.comparison_id));
  strategy.value = catalog.default_strategy;
  for (const [name, field] of Object.entries(schema.properties)) {
    const input = form.elements.namedItem(name);
    if (!input || field.default == null) continue;
    if (input.type === "checkbox") input.checked = field.default;
    else input.value = field.default;
  }
  const parameters = catalog.parameters.map(addStrategyParameter);
  function showStrategyFields() {
    const selected = catalog.strategies.find(item => item.id === strategy.value);
    for (const {field, input} of parameters) {
      const applies = strategy.value === catalog.comparison_id || selected?.parameter_ids.includes(input.name);
      field.hidden = !applies;
      input.disabled = !applies;
    }
  }
  strategy.addEventListener("change", showStrategyFields);
  showStrategyFields();
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
    if (!input.disabled && input.value !== "") options[input.name] = input.valueAsNumber;
  }
  el("start").disabled = true;
  try {
    const dataset = await request("/v1/datasets", {method:"POST", headers:{"Content-Type":"text/csv"}, body:file});
    const run = await request("/v1/runs", {method:"POST", headers:{"Content-Type":"application/json"}, body:JSON.stringify({dataset:dataset.id, options})});
    comparison.reset();
    active = run.id;
    el("cancel-error").textContent = "";
    el("cancel-run").hidden = true;
    const url = new URL(location.href);
    url.searchParams.set("run", active);
    history.replaceState(null,"",url);
    loadHistory();
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
