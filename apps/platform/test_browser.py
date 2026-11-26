"""Exercise the local workspace against the running API and worker in CI."""
import csv
import shutil
from pathlib import Path

from playwright.sync_api import expect, sync_playwright


with sync_playwright() as playwright:
    chrome = shutil.which("google-chrome") or shutil.which("chromium")
    browser = playwright.chromium.launch(executable_path=chrome)
    page = browser.new_page(viewport={"width": 390, "height": 844})
    page.goto("http://127.0.0.1:8765")
    page.get_by_label("Market data").set_input_files("data/sample_btc_1h.csv")
    page.get_by_label("Strategy", exact=True).select_option("all")
    page.get_by_label("Initial capital").fill("1800")
    page.get_by_label("Order amount").fill("10")
    page.get_by_label("Daily buy limit").fill("10")
    page.get_by_label("Minimum order").fill("1")
    page.get_by_role("button", name="Start backtest").click()
    expect(page.get_by_role("status")).to_have_text("succeeded", timeout=90000)
    assert "?run=" in page.url
    successful_url = page.url
    page.reload()
    ledger = page.get_by_role("link", name="Download order ledger")
    expect(ledger).to_be_visible()
    with page.expect_download() as received:
        ledger.click()
    assert Path(received.value.path()).read_bytes() == Path(".e2e/min.csv").read_bytes()
    expect(page.get_by_role("link", name="Download HTML report")).to_be_visible()
    expect(page.get_by_role("link", name="Download replay manifest")).to_be_visible()
    comparison = page.get_by_role("region", name="Strategy comparison")
    portfolios = comparison.get_by_role("table", name="Portfolio results")
    dca = portfolios.get_by_role("row").filter(
        has=page.get_by_role("rowheader", name="DCA", exact=True))
    dip = portfolios.get_by_role("row").filter(
        has=page.get_by_role("rowheader", name="DipDCA", exact=True))
    expect(dca.get_by_role("cell", name="$1,875.49", exact=True)).to_be_visible()
    expect(dip.get_by_role("cell", name="$1,862.67", exact=True)).to_be_visible()
    expect(dip.get_by_role("cell", name="-$12.82", exact=True)).to_be_visible()
    expect(dip.get_by_role("cell", name="$879.08", exact=True)).to_be_visible()
    expect(dip.get_by_role("cell", name="386 h", exact=True)).to_be_visible()
    expect(portfolios.locator('[aria-sort="descending"]')).to_contain_text("Final equity")
    portfolios.get_by_role("button", name="Sort by Portfolio", exact=True).click()
    expect(portfolios.locator('[aria-sort="ascending"]')).to_contain_text("Portfolio")
    expect(portfolios.get_by_role("rowheader").first).to_have_text("Buy and hold")
    expect(comparison.get_by_role("img", name="Equity over time")).to_be_visible()
    expect(comparison.get_by_role("list", name="Portfolio legend")).to_contain_text("DipDCA")
    expect(comparison.get_by_text("Bar timestamp (UTC)", exact=True)).to_be_visible()

    ledger_filter = comparison.get_by_label("Ledger portfolio")
    expect(ledger_filter).to_have_value("DCA")
    expect(comparison.locator("#order-range")).to_contain_text("Orders 1–25 of")
    comparison.get_by_role("button", name="Next orders").click()
    expect(comparison.locator("#order-range")).to_contain_text("Orders 26–50 of")
    ledger_filter.select_option("DipDCA")
    expect(comparison.locator("#order-range")).to_contain_text("Orders 1–25 of")
    with Path(".e2e/min.csv").open() as stream:
        dip_orders = [row for row in csv.DictReader(stream) if row["strategy"] == "DipDCA"]
    reasons = {"filled": "Filled", "daily_limit": "Skipped · Daily limit",
               "below_min_notional": "Skipped · Below minimum order",
               "insufficient_cash": "Skipped · Insufficient cash"}
    expected = [reasons[row["status"]] for row in dip_orders[:25]]
    assert any(reason.startswith("Skipped") for reason in expected)
    expect(comparison.locator("#order-table tbody tr td:nth-child(4)")).to_have_text(expected)
    ledger_filter.select_option("Cash")
    expect(comparison.locator("#order-range")).to_have_text("No orders for this portfolio.")
    expect(comparison.get_by_role("button", name="Next orders")).to_be_disabled()
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.get_by_label("Market data").set_input_files(
        {"name": "broken.csv", "mimeType": "text/csv", "buffer": b"bad\n1\n"})
    page.get_by_role("button", name="Start backtest").click()
    expect(page.get_by_role("status")).to_have_text("failed", timeout=90000)
    expect(page.locator("#diagnostic")).not_to_be_empty()
    expect(ledger).to_have_count(0)
    expect(comparison).to_be_hidden()
    page.goto("http://127.0.0.1:8765")
    history = page.get_by_role("region", name="Run history")
    older = history.locator(f'a[href="?{successful_url.split("?", 1)[1]}"]')
    older.click()
    expect(ledger).to_be_visible()
    assert page.url == successful_url
    expect(comparison.get_by_role("table", name="Portfolio results")).to_be_visible()

    newest = page.request.get("http://127.0.0.1:8765/v1/runs?limit=1").json()
    second = page.request.get(f"http://127.0.0.1:8765/v1/runs?limit=1&before={newest['next']}").json()
    assert newest["runs"][0]["state"] == "failed"
    assert second["runs"][0]["id"] == successful_url.split("run=")[1]
    browser.close()
