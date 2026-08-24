"""A containment both tiers would accept, refused by a guard that predates them.

`write_report_and_flush` is `write_report` plus one call, so the shared run is
six of its seven statements — 0.857 of the body, over the maximum run fraction.
A run covering nearly the whole body is the whole function again, which is the
symmetric detector's job. The shorter runs that would clear the fraction guard
drop the large trailing loop and fail the size-balance guard instead.

Nothing about the tiers changes that: the guards compose with them.
"""


def write_report(rows, out):
    header = header_for(rows)
    out.write(header)
    body = []
    total = 0
    for row in rows:
        body.append(format_row(row, total))
        total += row.amount
    for line in body:
        cleaned = line.strip().lower()
        if not cleaned:
            continue
        out.write(cleaned)
        out.write("\n")


def write_report_and_flush(rows, out):
    header = header_for(rows)
    out.write(header)
    body = []
    total = 0
    for row in rows:
        body.append(format_row(row, total))
        total += row.amount
    for line in body:
        cleaned = line.strip().lower()
        if not cleaned:
            continue
        out.write(cleaned)
        out.write("\n")
    out.flush()
