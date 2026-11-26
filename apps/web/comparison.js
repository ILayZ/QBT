const amount = new Intl.NumberFormat("en-US", {minimumFractionDigits: 2, maximumFractionDigits: 2});
const difference = new Intl.NumberFormat("en-US", {minimumFractionDigits: 2, maximumFractionDigits: 2, signDisplay: "exceptZero"});
const orderAmount = new Intl.NumberFormat("en-US", {minimumFractionDigits: 2, maximumFractionDigits: 8});
const percent = new Intl.NumberFormat("en-US", {style: "percent", maximumFractionDigits: 2});
const axisMoney = new Intl.NumberFormat("en-US", {maximumFractionDigits: 2});
const day = new Intl.DateTimeFormat("en-US", {month: "short", day: "numeric", year: "numeric", timeZone: "UTC"});
const clock = new Intl.DateTimeFormat("en-GB", {hour: "2-digit", minute: "2-digit", timeZone: "UTC"});
const pageSize = 25;
const columns = [
  ["name", "Portfolio", name => name],
  ["final_equity", "Final equity", value => amount.format(value)],
  ["difference_vs_dca", "vs DCA", value => difference.format(value)],
  ["max_drawdown", "Max drawdown", value => percent.format(value)],
  ["unspent_reserve", "Unspent reserve", value => amount.format(value)],
  ["max_reserve_wait_hours", "Max reserve wait", value => value + " h"],
];
const orderFields = [
  ["execution_time", "Execution (UTC)"], ["signal_time", "Signal (UTC)"], ["side", "Side"],
  ["status", "Status / reason"], ["price", "Price"], ["quantity", "Quantity"],
  ["fee", "Fee"], ["cash", "Cash after order"],
];
const orderStatuses = {
  filled: "Filled",
  daily_limit: "Skipped · Daily limit",
  below_min_notional: "Skipped · Below minimum order",
  insufficient_cash: "Skipped · Insufficient cash",
};
const colors = ["#00675f", "#92420b", "#3856a8", "#99447a", "#765700", "#555f66", "#7745a4"];
const dashes = ["", "7 3", "2 3", "9 3 2 3", "12 4", "4 4", "8 3 2 3 2 3"];

function node(tag, text) {
  const element = document.createElement(tag);
  if (text !== undefined) element.textContent = text;
  return element;
}

function svgNode(tag, attributes, content) {
  const element = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
  if (content !== undefined) element.textContent = content;
  return element;
}

async function artifact(url, signal, format) {
  const response = await fetch(url, {signal});
  if (!response.ok) throw new Error("HTTP " + response.status);
  return response[format]();
}

function requireV1(data) {
  if (data.version !== 1 || !Array.isArray(data.portfolios)) {
    throw new Error("Unsupported comparison artifact format.");
  }
  return data;
}

function drawSummary(table, portfolios) {
  let sortKey = "final_equity";
  let descending = true;
  const heading = table.tHead.rows[0];
  const body = table.tBodies[0];

  function render() {
    const rows = [...portfolios].sort((a, b) => {
      const left = a[sortKey], right = b[sortKey];
      if (left == null) return right == null ? 0 : 1;
      if (right == null) return -1;
      const comparison = typeof left === "string" ? left.localeCompare(right) : left - right;
      return descending ? -comparison : comparison;
    });
    body.replaceChildren();
    for (const row of rows) {
      const tr = node("tr");
      for (const [key, , format] of columns) {
        const cell = node(key === "name" ? "th" : "td", row[key] == null ? "—" : format(row[key]));
        if (key === "name") cell.scope = "row";
        tr.append(cell);
      }
      body.append(tr);
    }
    for (const [index, [key, label]] of columns.entries()) {
      const headingCell = heading.cells[index];
      if (key === sortKey) headingCell.setAttribute("aria-sort", descending ? "descending" : "ascending");
      else headingCell.removeAttribute("aria-sort");
      headingCell.firstChild.setAttribute("aria-label", "Sort by " + label);
      headingCell.firstChild.textContent = label + (key === sortKey ? (descending ? " ↓" : " ↑") : "");
    }
  }

  heading.replaceChildren();
  for (const [key, label] of columns) {
    const th = node("th"), button = node("button", label);
    th.scope = "col";
    button.type = "button";
    button.onclick = () => {
      descending = key === sortKey ? !descending : key !== "name";
      sortKey = key;
      render();
    };
    th.append(button);
    heading.append(th);
  }
  render();
}

function drawEquity(chart, legend, data, units) {
  const timestamps = data.timestamps.map(Date.parse);
  const start = timestamps[0], end = timestamps.at(-1);
  if (!timestamps.length || timestamps.some(time => !Number.isFinite(time)) || !data.portfolios.length) {
    throw new Error("Equity timestamps or portfolios are missing.");
  }
  const curves = data.portfolios.map(({name, values}, index) => {
    if (values.length !== timestamps.length || values.some(value => !Number.isFinite(value))) {
      throw new Error("Equity values do not match their timestamps.");
    }
    return {name, values, color: colors[index % colors.length], dash: dashes[index % dashes.length]};
  });
  let low = Infinity, high = -Infinity;
  for (const {values} of curves) {
    for (const value of values) {
      low = Math.min(low, value);
      high = Math.max(high, value);
    }
  }
  const padding = (high - low) * 0.05 || Math.max(Math.abs(low) * 0.01, 1);
  low -= padding;
  high += padding;
  legend.replaceChildren();
  for (const {name, color, dash} of curves) {
    const item = node("li");
    const key = svgNode("svg", {width: 30, height: 12, "aria-hidden": "true"});
    key.append(svgNode("path", {d: "M1 6H29", stroke: color, "stroke-width": 2, "stroke-dasharray": dash}));
    item.append(key, node("span", name));
    legend.append(item);
  }

  function draw(width) {
    if (width < 120) return;
    const height = 270, left = 77, right = width - 12, top = 25, bottom = 205;
    const x = time => start === end ? (left + right) / 2 : left + (time - start) / (end - start) * (right - left);
    const y = value => bottom - (value - low) / (high - low) * (bottom - top);
    const axisLabel = (x, y, text, anchor = "start") => svgNode("text", {x, y, "text-anchor": anchor}, text);
    chart.setAttribute("viewBox", "0 0 " + width + " " + height);
    const title = svgNode("title", {id: "equity-chart-title"}, "Equity over time");
    const desc = svgNode("desc", {id: "equity-chart-description"},
      "Portfolio equity in " + units + " from " + new Date(start).toISOString() + " through " + new Date(end).toISOString() +
      ". Vertical axis ranges from " + amount.format(low) + " to " + amount.format(high) + ".");
    chart.replaceChildren(title, desc, axisLabel(left, 14, "Equity (" + units + ")"));
    for (let i = 0; i <= 4; i++) {
      const value = low + (high - low) * i / 4, row = y(value);
      chart.append(svgNode("line", {x1: left, x2: right, y1: row, y2: row, class: "chart-grid"}),
        axisLabel(left - 6, row + 4, axisMoney.format(value), "end"));
    }
    chart.append(svgNode("line", {x1: left, x2: left, y1: top, y2: bottom, class: "chart-grid"}));
    const ticks = start === end ? 1 : width > 600 ? 4 : 2;
    for (let i = 0; i < ticks; i++) {
      const time = ticks === 1 ? start : start + (end - start) * i / (ticks - 1);
      const position = x(time), anchor = ticks === 1 ? "middle" : i === 0 ? "start" : i === ticks - 1 ? "end" : "middle";
      chart.append(axisLabel(position, 222, day.format(time), anchor),
        axisLabel(position, 237, clock.format(time), anchor));
    }
    chart.append(axisLabel((left + right) / 2, 259, "Bar timestamp (UTC)", "middle"));
    for (const {values, color, dash} of curves) {
      if (values.length === 1) {
        chart.append(svgNode("circle", {cx: x(start), cy: y(values[0]), r: 3, fill: color}));
        continue;
      }
      const points = [];
      const groupSize = Math.max(1, Math.floor(values.length / (right - left)));
      function add(index) {
        points.push(x(timestamps[index]).toFixed(1) + "," + y(values[index]).toFixed(1));
      }
      add(0);
      // Keep local highs and lows in time order so narrow spikes remain visible.
      for (let from = 1; from < values.length - 1; from += groupSize) {
        let min = from, max = from;
        const to = Math.min(from + groupSize, values.length - 1);
        for (let i = from + 1; i < to; i++) {
          if (values[i] < values[min]) min = i;
          if (values[i] > values[max]) max = i;
        }
        add(Math.min(min, max));
        if (min !== max) add(Math.max(min, max));
      }
      add(values.length - 1);
      chart.append(svgNode("polyline", {points: points.join(" "), fill: "none", stroke: color,
        "stroke-width": 2, "stroke-dasharray": dash}));
    }
  }
  const observer = new ResizeObserver(([entry]) => draw(Math.floor(entry.contentRect.width)));
  observer.observe(chart);
  return () => observer.disconnect();
}

function readOrders(csv) {
  // The v1 CLI writes fixed, unquoted fields with no embedded commas.
  const [header, ...lines] = csv.trimEnd().split(/\r?\n/);
  const keys = header.split(",");
  if (!["strategy", ...orderFields.map(([key]) => key)].every(key => keys.includes(key))) {
    throw new Error("Unsupported order ledger format.");
  }
  const strategyIndex = keys.indexOf("strategy");
  const orderIndices = orderFields.map(([key]) => keys.indexOf(key));
  const strategies = new Map();
  for (const line of lines) {
    const row = line.split(",");
    const strategy = row[strategyIndex];
    if (!strategies.has(strategy)) strategies.set(strategy, []);
    strategies.get(strategy).push(orderIndices.map(index => row[index]));
  }
  return strategies;
}

function drawOrders(root, strategies, portfolios) {
  const dropdown = root.querySelector("#order-strategy");
  const status = root.querySelector("#order-status");
  const previous = root.querySelector("#order-previous"), next = root.querySelector("#order-next");
  const range = root.querySelector("#order-range");
  const body = root.querySelector("#order-table tbody");
  const heading = root.querySelector("#order-table thead tr");
  let page = 0;
  let filtered = [];
  const statusIndex = orderFields.findIndex(([key]) => key === "status");
  dropdown.replaceChildren();
  for (const name of new Set([...portfolios.map(row => row.name), ...strategies.keys()])) {
    dropdown.add(new Option(name, name));
  }
  if ([...dropdown.options].some(option => option.value === "DCA")) dropdown.value = "DCA";
  heading.replaceChildren();
  for (const [, label] of orderFields) {
    const cell = node("th", label);
    cell.scope = "col";
    heading.append(cell);
  }

  function render() {
    const rows = filtered;
    const first = page * pageSize, last = Math.min(first + pageSize, rows.length);
    body.replaceChildren();
    for (const row of rows.slice(first, last)) {
      const tr = node("tr");
      for (const [index, value] of row.entries()) {
        const key = orderFields[index][0];
        let display = value;
        if (key.endsWith("_time")) display = value.replace("T", " ").replace(/(?:\+00:00|Z)$/, "");
        if (key === "status") display = Object.hasOwn(orderStatuses, value) ? orderStatuses[value] : value;
        if (key === "price" || key === "fee" || key === "cash") display = orderAmount.format(Number(value));
        tr.append(node("td", display));
      }
      body.append(tr);
    }
    range.textContent = rows.length ? `Orders ${first + 1}–${last} of ${rows.length}` : "No orders match the selected filters.";
    previous.disabled = page === 0;
    next.disabled = last >= rows.length;
  }
  function filter() {
    const rows = strategies.get(dropdown.value) || [];
    filtered = status.value ? rows.filter(row => row[statusIndex] === status.value) : rows;
    page = 0;
    render();
  }
  status.replaceChildren(new Option("All statuses", ""));
  for (const [value, label] of Object.entries(orderStatuses)) status.add(new Option(label, value));
  dropdown.onchange = filter;
  status.onchange = filter;
  previous.onclick = () => { page--; render(); };
  next.onclick = () => { page++; render(); };
  filter();
}

export function createComparison(root) {
  const message = root.querySelector("#comparison-message");
  const content = root.querySelector("#comparison-content");
  let current;
  let stopChart;
  function reset() {
    current?.abort();
    current = undefined;
    stopChart?.();
    stopChart = undefined;
    root.hidden = true;
    content.hidden = true;
  }
  async function show(run) {
    reset();
    if (run.state !== "succeeded") return;
    root.querySelector("#comparison-title").textContent = run.options.strategy === "all" ? "Strategy comparison" : "Portfolio performance";
    root.hidden = false;
    const {["summary.json"]: summary, ["equity.json"]: equity, ["orders.csv"]: orders} = run.artifacts;
    if (!summary || !equity || !orders) {
      message.textContent = "Comparison preview is unavailable for this run. See Run results for available downloads.";
      return;
    }
    message.textContent = "Loading comparison…";
    const controller = new AbortController();
    current = controller;
    const {signal} = controller;
    try {
      const [metrics, series, csv] = await Promise.all([
        artifact(summary, signal, "json"), artifact(equity, signal, "json"), artifact(orders, signal, "text"),
      ]);
      if (signal.aborted) return;
      const {portfolios, quote_currency: quote} = requireV1(metrics);
      const units = typeof quote === "string" && quote.trim() ? quote.trim() : "quote units";
      root.querySelector("#comparison-units").textContent = "Monetary values in " + units + ".";
      const orderRows = readOrders(csv);
      drawSummary(root.querySelector("#portfolio-table"), portfolios);
      stopChart = drawEquity(root.querySelector("#equity-chart"), root.querySelector("#equity-legend"), requireV1(series), units);
      drawOrders(root, orderRows, portfolios);
      message.textContent = "";
      content.hidden = false;
    } catch (err) {
      if (signal.aborted) return;
      message.textContent = "Could not load comparison. " + err.message;
    }
  }
  return {reset, show};
}
