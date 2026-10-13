import SwiftUI

/// A provider's own mark beside its name, as the console's ProviderMark shows it
/// (ui/src/components/ProviderMark.tsx): Anthropic's in the text colour, Kimi Code's in its
/// light and dark files, and a provider with no mark — OpenAI's — by its name in words.
struct ProviderMark: View {
    enum Form {
        /// The mark and the provider's name.
        case named
        /// The mark alone, its name for VoiceOver; the name in words when there is no mark.
        case alone
        /// The mark beside other words: "active for Kimi Code".
        case words(String)
    }

    let provider: String?
    var form: Form = .named
    @ScaledMetric(relativeTo: .caption) private var size: CGFloat = 13

    /// The asset that holds a provider's mark, or nil for one that has none.
    nonisolated static func asset(_ provider: String?) -> String? {
        switch provider {
        case nil, "anthropic": "ProviderAnthropic"
        case "kimi": "ProviderKimi"
        default: nil
        }
    }

    var body: some View {
        let name = Provider.name(provider)
        HStack(spacing: 4) {
            if let asset = Self.asset(provider) {
                Image(asset)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size, height: size)
                    .foregroundStyle(.primary)
            }
            switch form {
            case .named: Text(name)
            case .words(let w): Text(w)
            case .alone: if Self.asset(provider) == nil { Text(name) }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label(name))
    }

    private func label(_ name: String) -> String {
        if case .words(let w) = form { return w }
        return name
    }
}
