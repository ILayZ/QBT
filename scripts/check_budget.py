"""Reconcile the multi-strategy sample run against its journal and printed report."""

import csv
import math
import re
import sys
from collections import defaultdict
from pathlib import Path


def check(data, journal, report, include_fees):
    with open(data) as stream:
        final_close = float(list(csv.DictReader(stream))[-1]["close"])
    with open(journal) as stream:
        orders = defaultdict(list)
        for row in csv.DictReader(stream):
            orders[row["strategy"]].append(row)
    html = Path(report).read_text()
    assert orders.keys() == {"DCA", "DipDCA", "VolAdjustedDCA", "MomentumSMA", "RegimeSwitch"}
    for name, rows in orders.items():
        cash, units, fees, buys = 1800.0, 0.0, 0.0, 0
        daily = defaultdict(float)
        for row in rows:
            qty, price, fee = (float(row[k]) for k in ("quantity", "price", "fee"))
            if row["status"] == "filled":
                assert qty > 0 and row["execution_time"] > row["signal_time"]
                value = qty * price
                assert math.isclose(fee, value * 0.001)
                fees += fee
                if row["side"] == "buy":
                    cash -= value + fee
                    units += qty
                    buys += 1
                    daily[row["execution_time"][:10]] += value + (fee if include_fees else 0)
                else:
                    cash += value - fee
                    units -= qty
            else:
                assert qty == fee == 0
            assert math.isclose(cash, float(row["cash"]), abs_tol=1e-8)
            assert cash >= -1e-8 and units >= -1e-10
        assert max(daily.values()) <= 10 + 1e-9
        section = html.split(f"<h2>{name}</h2>", 1)[1].split("</table>", 1)[0]
        displayed = float(re.search(r"Final Value</b></td><td>\$([\d.]+)", section)[1])
        assert abs(cash + units * final_close - displayed) <= 0.005001
        if name == "DCA" and not include_fees:
            assert buys == 179 and round(fees, 2) == 1.79
            assert round(cash, 2) == 8.21 and displayed == 1875.49
        if name == "DCA" and include_fees:
            assert buys == 180 and abs(cash) < 1e-8
        print(f"{name}: buys={buys} fees={fees:.2f} cash={cash:.2f} final={displayed:.2f}")


if __name__ == "__main__":
    check(*sys.argv[1:4], include_fees=sys.argv[4] == "inclusive")
