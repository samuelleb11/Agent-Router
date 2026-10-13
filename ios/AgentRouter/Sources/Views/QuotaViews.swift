import Charts
import SwiftUI

/// A window as a filled bar with what it means underneath: the reading's own figure, and the
/// forecast's projection for the same window.
struct QuotaRow: View {
    let meter: QuotaMeter

    var body: some View {
        let live = meter.used != nil
        let used = meter.used ?? 0
        VStack(alignment: .leading, spacing: 6) {
            Gauge(value: min(1, max(0, used)), in: 0...1) {
                Text(meter.label)
            } currentValueLabel: {
                Text(live ? Fmt.percent(used) : "—")
            }
            .gaugeStyle(.linearCapacity)
            .tint(QuotaColor.tint(used, live: live))
            if let expired = meter.expired {
                Text(expired)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if let reset = meter.resetsAt {
                Text("resets \(Fmt.relative(reset)) · \(Fmt.when(reset))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if meter.expired == nil, let p = Projection.describe(meter.forecast) {
                Label {
                    Text(p.text)
                } icon: {
                    if p.tone == .bad { Image(systemName: "exclamationmark.triangle.fill") }
                }
                .font(.caption)
                .foregroundStyle(color(p.tone))
                if let basis = p.basis {
                    Text(basis)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    private func color(_ tone: Projection.Tone) -> Color {
        switch tone {
        case .plain: .secondary
        case .warn: .orange
        case .bad: .red
        }
    }
}

/// Observed utilisation across the window's life, then the projection to its end.
struct BurnDownChart: View {
    let window: Forecast.Window

    private struct Sample: Identifiable {
        let id = UUID()
        let date: Date
        let used: Double
        let projected: Bool
    }

    private var samples: [Sample] {
        guard let start = window.startDate, let reset = window.resetDate else { return [] }
        let now = min(Date.now, reset)
        let used = window.used ?? 0
        var out = (window.series ?? []).map { Sample(date: $0.date, used: $0.u, projected: false) }
        if out.isEmpty { out.append(Sample(date: start, used: 0, projected: false)) }
        out.append(Sample(date: now, used: used, projected: false))
        out.append(Sample(date: now, used: used, projected: true))
        if let at = window.exhaustDate {
            out.append(Sample(date: at, used: 1, projected: true))
        } else {
            out.append(Sample(date: reset, used: window.projectedEndUtil ?? used, projected: true))
        }
        return out
    }

    var body: some View {
        let s = samples
        Chart {
            RuleMark(y: .value("Limit", 1.0))
                .foregroundStyle(.quaternary)
            ForEach(s) { p in
                LineMark(x: .value("Time", p.date), y: .value("Used", p.used), series: .value("Kind", p.projected ? "projected" : "used"))
                    .foregroundStyle(p.projected ? (window.exhaustsAt != nil ? Color.red : Color.secondary) : Color.accentColor)
                    .lineStyle(StrokeStyle(lineWidth: 2, dash: p.projected ? [4, 3] : []))
                    .interpolationMethod(.linear)
            }
            if let at = window.exhaustDate {
                PointMark(x: .value("Time", at), y: .value("Used", 1.0))
                    .foregroundStyle(.red)
                    .symbolSize(40)
            }
        }
        .chartYScale(domain: 0...1)
        .chartYAxis {
            AxisMarks(values: [0, 0.5, 1]) { v in
                AxisGridLine()
                AxisValueLabel { if let d = v.as(Double.self) { Text(Fmt.percent(d)) } }
            }
        }
        .chartXAxis {
            AxisMarks(values: .automatic(desiredCount: 3)) { _ in
                AxisGridLine()
                AxisValueLabel(format: .dateTime.month(.abbreviated).day().hour())
            }
        }
        .chartLegend(.hidden)
        .frame(height: 120)
        .accessibilityLabel("Burn-down chart")
        .accessibilityValue(Projection.describe(window)?.text ?? "")
    }
}
