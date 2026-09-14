import Charts
import SwiftUI

/// The dashboard's model-family colours, one hue per family.
enum FamilyColor {
    static let order = ["opus", "sonnet", "haiku", "other"]
    static func of(_ family: String) -> Color {
        switch family {
        case "opus": .purple
        case "sonnet": .orange
        case "haiku": .teal
        default: .brown
        }
    }
}

/// Prompt tokens per bucket, stacked by model family — the activity chart.
struct ActivityChart: View {
    let buckets: [Bucket]
    let bucketMs: Int

    private struct Slice: Identifiable {
        let date: Date
        let family: String
        let tokens: Double
        var id: String { "\(date.timeIntervalSince1970)-\(family)" }
    }

    private var slices: [Slice] {
        buckets.flatMap { b in
            FamilyColor.order.compactMap { f in
                let v = b.families[f] ?? 0
                return v > 0 ? Slice(date: b.date, family: f, tokens: v) : nil
            }
        }
    }

    var body: some View {
        let s = slices
        VStack(alignment: .leading, spacing: 6) {
            Chart(s) { p in
                BarMark(x: .value("Time", p.date, unit: unit), y: .value("Prompt tokens", p.tokens))
                    .foregroundStyle(by: .value("Model", p.family))
            }
            .chartForegroundStyleScale(domain: FamilyColor.order, range: FamilyColor.order.map(FamilyColor.of))
            .chartYAxis {
                AxisMarks(values: .automatic(desiredCount: 3)) { v in
                    AxisGridLine()
                    AxisValueLabel { if let d = v.as(Double.self) { Text(Fmt.compact(d)) } }
                }
            }
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: 4)) { _ in AxisGridLine(); AxisValueLabel(format: xFormat) } }
            .chartLegend(.hidden)
            .frame(height: 150)
            HStack(spacing: 12) {
                ForEach(FamilyColor.order, id: \.self) { f in
                    HStack(spacing: 4) {
                        Circle().fill(FamilyColor.of(f)).frame(width: 8, height: 8)
                        Text(f)
                    }
                    .opacity(s.contains { $0.family == f } ? 1 : 0.4)
                }
                Spacer()
                Text("prompt tokens per \(Fmt.bucketName(bucketMs))")
                    .foregroundStyle(.secondary)
            }
            .font(.caption2)
        }
        .accessibilityLabel("Activity over time")
    }

    private var unit: Calendar.Component {
        bucketMs >= 86_400_000 ? .day : bucketMs >= 3_600_000 ? .hour : .minute
    }
    private var xFormat: Date.FormatStyle {
        bucketMs >= 86_400_000 ? .dateTime.month(.abbreviated).day() : .dateTime.hour().minute()
    }
}

/// A small line of recent values, the dashboard's sparkline.
struct Sparkline: View {
    let values: [Double]
    var body: some View {
        Chart(Array(values.enumerated()), id: \.offset) { i, v in
            AreaMark(x: .value("i", i), y: .value("v", v))
                .foregroundStyle(Color.accentColor.opacity(0.18))
            LineMark(x: .value("i", i), y: .value("v", v))
                .foregroundStyle(Color.accentColor)
                .lineStyle(StrokeStyle(lineWidth: 1.5))
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .frame(height: 28)
        .accessibilityHidden(true)
    }
}

/// Proportional segments in one bar, with a legend — the dashboard's `.stack`.
struct ShareBar: View {
    struct Segment: Identifiable {
        let key: String
        let label: String
        let value: Double
        let color: Color
        var id: String { key }
    }
    let segments: [Segment]

    private var total: Double { segments.reduce(0) { $0 + $1.value } }

    var body: some View {
        let live = segments.filter { $0.value > 0 }
        VStack(alignment: .leading, spacing: 8) {
            GeometryReader { geo in
                HStack(spacing: 2) {
                    ForEach(live) { s in
                        RoundedRectangle(cornerRadius: 3)
                            .fill(s.color)
                            .frame(width: max(2, geo.size.width * s.value / max(total, 1) - 2))
                    }
                }
            }
            .frame(height: 12)
            .accessibilityHidden(true)
            ForEach(segments) { s in
                HStack(spacing: 8) {
                    Circle().fill(s.color).frame(width: 8, height: 8)
                    Text(s.label)
                    Spacer()
                    Text(Fmt.compact(s.value)).monospacedDigit()
                    Text(Fmt.share(total > 0 ? s.value / total : 0))
                        .foregroundStyle(.secondary)
                        .frame(width: 44, alignment: .trailing)
                        .monospacedDigit()
                }
                .font(.caption)
                .opacity(s.value > 0 ? 1 : 0.45)
            }
        }
    }
}

/// A row with a label, a value, and a bar sized against the largest in its list.
struct BarRow<Leading: View>: View {
    let value: Double
    let max: Double
    let color: Color
    let text: String
    @ViewBuilder let leading: () -> Leading

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                leading()
                Spacer(minLength: 8)
                Text(text).font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            GeometryReader { geo in
                RoundedRectangle(cornerRadius: 3)
                    .fill(color.opacity(0.85))
                    .frame(width: geo.size.width * (max > 0 ? min(1, value / max) : 0))
            }
            .frame(height: 6)
            .background(RoundedRectangle(cornerRadius: 3).fill(color.opacity(0.12)))
            .accessibilityHidden(true)
        }
        .padding(.vertical, 2)
    }
}

/// A headline number with a caption, the dashboard's tile.
struct StatTile<Extra: View>: View {
    let label: String
    let value: String
    let sub: String
    var bad = false
    @ViewBuilder let extra: () -> Extra

    init(_ label: String, _ value: String, _ sub: String, bad: Bool = false, @ViewBuilder extra: @escaping () -> Extra = { EmptyView() }) {
        self.label = label; self.value = value; self.sub = sub; self.bad = bad; self.extra = extra
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            Text(value)
                .font(.title3.weight(.semibold))
                .monospacedDigit()
                .foregroundStyle(bad ? Color.red : Color.primary)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Text(sub).font(.caption2).foregroundStyle(.secondary).lineLimit(2)
            extra()
        }
        // Fill the grid row, so a tile beside a taller one does not float in a gap.
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemGroupedBackground)))
        .accessibilityElement(children: .combine)
    }
}
