#import "daily-usage-chart.typ": daily-usage-chart, daily-usage-report-width

#let source = sys.inputs.at("source", default: "history.csv")
#let display-days = int(sys.inputs.at("days", default: "183"))
#let report-width = daily-usage-report-width(
    source,
    display-days: display-days,
)

#set page(
    width: report-width,
    height: auto,
    margin: 12mm,
    fill: rgb("#ffffff"),
)

#daily-usage-chart(source, display-days: display-days)
