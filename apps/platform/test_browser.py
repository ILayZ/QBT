"""Exercise the local workspace against the running API and worker in CI."""
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
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.get_by_label("Market data").set_input_files(
        {"name": "broken.csv", "mimeType": "text/csv", "buffer": b"bad\n1\n"})
    page.get_by_role("button", name="Start backtest").click()
    expect(page.get_by_role("status")).to_have_text("failed", timeout=90000)
    expect(page.locator("#diagnostic")).not_to_be_empty()
    expect(ledger).to_have_count(0)
    page.goto("http://127.0.0.1:8765")
    history = page.get_by_role("region", name="Run history")
    older = history.locator(f'a[href="?{successful_url.split("?", 1)[1]}"]')
    older.click()
    expect(ledger).to_be_visible()
    assert page.url == successful_url

    newest = page.request.get("http://127.0.0.1:8765/v1/runs?limit=1").json()
    second = page.request.get(f"http://127.0.0.1:8765/v1/runs?limit=1&before={newest['next']}").json()
    assert newest["runs"][0]["state"] == "failed"
    assert second["runs"][0]["id"] == successful_url.split("run=")[1]
    browser.close()
