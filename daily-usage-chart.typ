#import "@preview/lilaq:0.6.0" as lq

// The source only guarantees a numeric value. Keep units out of calculations
// and presentation so histories from differently labelled queries remain valid.
#let parse-query-date(value) = {
    let parts = value.trim().split("-")
    assert(
        parts.len() == 3,
        message: "Expected a date in YYYY-MM-DD format, got: " + value,
    )
    datetime(
        year: int(parts.at(0)),
        month: int(parts.at(1)),
        day: int(parts.at(2)),
    )
}

#let format-value(value, digits: 2) = {
    str(calc.round(value, digits: digits))
}

#let load-electricity-samples(source) = {
    let rows = csv(source, row-type: dictionary)
    assert(
        rows.len() >= 2,
        message: "At least two electricity samples are required",
    )

    let samples = rows.map(row => (
        date: parse-query-date(row.at("date")),
        electricity: float(row.at("electricity").trim()),
    ))

    for index in range(1, samples.len()) {
        assert(
            samples.at(index).date > samples.at(index - 1).date,
            message: "CSV rows must be ordered by unique, ascending dates",
        )
    }
    samples
}

// Expand each query interval to one point per calendar day. Missing-query days
// are explicitly marked as estimates; the queried endpoint is always retained.
#let expand-daily-series(samples) = {
    let daily = ()
    for index in range(1, samples.len()) {
        let previous = samples.at(index - 1)
        let current = samples.at(index)
        let elapsed-days = int((current.date - previous.date).days())
        let daily-change = (
            previous.electricity - current.electricity
        ) / elapsed-days

        for offset in range(1, elapsed-days + 1) {
            let is-endpoint = offset == elapsed-days
            daily.push((
                date: previous.date + duration(days: offset),
                value: calc.round(daily-change, digits: 2),
                remaining: if is-endpoint {
                    current.electricity
                } else {
                    calc.round(
                        previous.electricity - daily-change * offset,
                        digits: 2,
                    )
                },
                change-estimated: elapsed-days > 1,
                balance-estimated: not is-endpoint,
            ))
        }
    }
    daily
}

#let load-daily-net-usage(source) = {
    expand-daily-series(load-electricity-samples(source))
}

#let select-latest(points, display-days) = {
    assert(
        type(display-days) == int and display-days > 0,
        message: "display-days must be a positive integer",
    )
    if points.len() <= display-days {
        points
    } else {
        points.slice(points.len() - display-days)
    }
}

// Public sizing helper used by standalone render documents. The plot grows by
// one fixed slot per day instead of hiding or sampling dates.
#let daily-usage-plot-width(
    source,
    display-days: 183,
    min-width: 20cm,
    day-width: 11mm,
) = {
    let count = calc.min(
        load-daily-net-usage(source).len(),
        display-days,
    )
    calc.max(min-width, count * day-width)
}

#let daily-usage-report-width(
    source,
    display-days: 183,
    min-width: 20cm,
    day-width: 11mm,
) = {
    daily-usage-plot-width(
        source,
        display-days: display-days,
        min-width: min-width,
        day-width: day-width,
    ) + 34mm
}

#let metric-card(label, value, detail: none) = {
    block(
        width: 100%,
        height: 20mm,
        inset: (x: 4mm, y: 3mm),
        radius: 2mm,
        fill: rgb("#f4f7fb"),
        stroke: .5pt + rgb("#e5eaf1"),
    )[
        #text(size: 7pt, weight: "semibold", fill: rgb("#758094"), label)
        #v(1.2mm)
        #text(size: 14pt, weight: "bold", fill: rgb("#172033"), value)
        #if detail != none {
            h(1.5mm)
            text(size: 7pt, fill: rgb("#8a94a6"), detail)
        }
    ]
}

#let section-heading(title, color: rgb("#172033")) = {
    block(width: 100%)[
        #text(size: 12pt, weight: "bold", fill: color, title)
    ]
}

// A compact dashboard with one combined movement chart and a marked line for
// the remaining-value trend. The two bar series use color-coded y-axes.
// Every calendar day receives a tick, mark, and numeric label.
#let daily-usage-chart(
    source,
    display-days: 183,
    min-width: 20cm,
    day-width: 11mm,
    bar-height: 72mm,
    line-height: 66mm,
    decrease-fill: rgb("#4676e8"),
    increase-fill: rgb("#e99845"),
    line-color: rgb("#168f88"),
) = {
    let all-points = load-daily-net-usage(source)
    let points = select-latest(all-points, display-days)
    let plot-width = calc.max(min-width, points.len() * day-width)
    let dates = points.map(point => point.date)
    let changes = points.map(point => point.value)
    let balances = points.map(point => point.remaining)
    let bar-values = changes.map(value => calc.max(value, 0))
    let recharge-values = changes.map(value => calc.max(-value, 0))
    let date-ticks = dates.map(date => (
        date,
        text(
            size: 6.5pt,
            fill: rgb("#6f7a8d"),
            date.display("[month]-[day]"),
        ),
    ))

    let bar-max = calc.max(..bar-values)
    let bar-span = calc.max(bar-max, 1)
    let change-limits = (0, bar-max + bar-span * 0.30)
    let balance-min = calc.min(..balances)
    let balance-max = calc.max(..balances)
    let balance-span = calc.max(balance-max - balance-min, 1)
    let balance-limits = (
        balance-min - balance-span * 0.20,
        balance-max + balance-span * 0.22,
    )

    let decreased-days = changes.filter(value => value >= 0)
    let average-drop = if decreased-days.len() == 0 {
        0
    } else {
        decreased-days.fold(0, (sum, value) => sum + value) / decreased-days.len()
    }
    let balance-up-windows = changes.filter(value => value < 0).len()
    let estimated-days = points.filter(point => point.change-estimated).len()
    let first-date = dates.first().display("[year]-[month]-[day]")
    let last-date = dates.last().display("[year]-[month]-[day]")
    let latest-balance = balances.last()

    block(width: plot-width)[
        #text(
            size: 19pt,
            weight: "bold",
            fill: rgb("#172033"),
            [Daily balance overview],
        )

        #v(5mm)
        #block(width: calc.min(plot-width, 212mm))[
            #grid(
                columns: (1fr, 1fr, 1fr, 1fr),
                gutter: 3mm,
                metric-card([DATE RANGE], [#first-date], detail: [to #last-date]),
                metric-card([LATEST VALUE], [#format-value(latest-balance)]),
                metric-card([AVG DAILY DROP], [#format-value(average-drop)]),
                metric-card([BALANCE-UP DAYS], [#balance-up-windows]),
            )
        ]

        #v(7mm)
        #section-heading([Daily balance movement])
        #v(1mm)
        #text(size: 7.5pt, weight: "semibold", fill: decrease-fill)[■ Decrease]
        #h(4mm)
        #text(size: 7.5pt, weight: "semibold", fill: increase-fill)[■ Increase]
        #v(2mm)
        #lq.diagram(
            width: plot-width,
            height: bar-height,
            legend: none,
            fill: rgb("#fbfcfe"),
            grid: (stroke: .45pt + rgb("#e7ebf2")),
            xlim: (
                dates.first() - duration(hours: 18),
                dates.last() + duration(hours: 18),
            ),
            ylim: change-limits,
            xaxis: (
                ticks: date-ticks,
                subticks: none,
                mirror: false,
            ),
            yaxis: (
                subticks: none,
                mirror: false,
                exponent: none,
            ),
            margin: (x: 0%, y: 8%),
            lq.bar(
                dates,
                bar-values,
                width: duration(hours: 16),
                fill: decrease-fill,
            ),
            ..points.map(point => {
                if point.value < 0 { return }
                let suffix = if point.change-estimated { "*" } else { "" }
                let label = text(
                    size: 6.5pt,
                    weight: "semibold",
                    fill: if point.change-estimated {
                        rgb("#8a94a6")
                    } else {
                        rgb("#3d475a")
                    },
                    format-value(point.value) + suffix,
                )
                lq.place(
                    point.date,
                    point.value,
                    pad(bottom: 2pt, label),
                    align: bottom,
                )
            }),
            if balance-up-windows > 0 {
                lq.yaxis(
                    position: right,
                    subticks: none,
                    mirror: false,
                    exponent: none,
                    stroke: increase-fill,
                    lq.bar(
                        dates,
                        recharge-values,
                        width: duration(hours: 16),
                        fill: increase-fill,
                    ),
                    ..points.filter(point => point.value < 0).map(point => {
                        let suffix = if point.change-estimated { "*" } else { "" }
                        let label = text(
                            size: 7pt,
                            weight: "bold",
                            fill: increase-fill.darken(25%),
                            "+" + format-value(-point.value) + suffix,
                        )
                        lq.place(
                            point.date,
                            -point.value,
                            pad(bottom: 2pt, label),
                            align: bottom,
                        )
                    }),
                )
            },
        )

        #v(7mm)
        #section-heading([Remaining balance], color: line-color)
        #v(2mm)
        #lq.diagram(
            width: plot-width,
            height: line-height,
            legend: none,
            fill: rgb("#fbfcfe"),
            grid: (stroke: .45pt + rgb("#e7ebf2")),
            xlim: (
                dates.first() - duration(hours: 18),
                dates.last() + duration(hours: 18),
            ),
            ylim: balance-limits,
            xaxis: (
                ticks: date-ticks,
                subticks: none,
                mirror: false,
            ),
            yaxis: (
                subticks: none,
                mirror: false,
                exponent: none,
            ),
            margin: (x: 0%, y: 0%),
            lq.plot(
                dates,
                balances,
                color: line-color,
                stroke: 1.25pt,
                mark: "o",
                mark-size: 3.2pt,
                clip: false,
            ),
            ..points.enumerate().map(((index, point)) => {
                let above = calc.even(index)
                let suffix = if point.balance-estimated { "*" } else { "" }
                let label = text(
                    size: 6.2pt,
                    weight: "semibold",
                    fill: if point.balance-estimated {
                        rgb("#98a1b1")
                    } else {
                        rgb("#246f6b")
                    },
                    format-value(point.remaining) + suffix,
                )
                let body = if above {
                    pad(bottom: 3pt, label)
                } else {
                    pad(top: 3pt, label)
                }
                lq.place(
                    point.date,
                    point.remaining,
                    body,
                    align: if above { bottom } else { top },
                )
            }),
        )

        #if estimated-days > 0 {
            v(3mm)
            text(size: 7pt, fill: rgb("#8a94a6"), [\* Estimated])
        }
    ]
}
