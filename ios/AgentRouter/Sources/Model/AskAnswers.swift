// Turning what was picked into what the tool expects — and into what a person would
// have typed, for when the prompt cannot be answered directly.

import Foundation

enum AskAnswers {
    struct Selection: Equatable, Sendable {
        var chosen: Set<String> = []
        var other: String = ""
    }

    /// `question text -> answer`, plus one line per answered question. Option order is
    /// the tool's own; "Other" is whatever was typed, and typing it is choosing it.
    static func compose(_ questions: [AskQuestion], _ selections: [String: Selection])
        -> (answers: [String: String], parts: [String]) {
        var answers: [String: String] = [:]
        var parts: [String] = []
        for q in questions {
            guard let sel = selections[q.question] else { continue }
            var picked = (q.options ?? []).map(\.label).filter { sel.chosen.contains($0) }
            let other = sel.other.trimmingCharacters(in: .whitespacesAndNewlines)
            if !other.isEmpty { picked.append(other) }
            guard !picked.isEmpty else { continue }
            let a = picked.joined(separator: ", ")
            answers[q.question] = a
            parts.append("\(q.title): \(a)")
        }
        return (answers, parts)
    }
}
