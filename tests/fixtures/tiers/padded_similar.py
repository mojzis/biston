"""Eleven raw lines each, four executable lines each.

Everything else is comments and blank lines. The two differ in one literal, so
they score 0.875 — above the default threshold. A floor measured on the raw
line span would report this pair; a floor measured on executable lines is not
fooled by padding, which is the reason the measure exists.
"""


def collect_names(rows):
    # Pull the names out of the rows.

    # Nothing subtle happens here, but the comments are long enough
    # that the function looks substantial from a line count alone.
    names = sorted({row.name.strip().lower() for row in rows if row.name})
    counted = len(names)

    # And return them.

    return [name for name in names if counted > 0]


def collect_labels(rows):
    # Pull the labels out of the rows.

    # Again padded with prose so the raw span clears ten lines,
    # while the executable content stays at four lines.
    names = sorted({row.name.strip().lower() for row in rows if row.name})
    counted = len(names)

    # And return them.

    return [name for name in names if counted > 1]
